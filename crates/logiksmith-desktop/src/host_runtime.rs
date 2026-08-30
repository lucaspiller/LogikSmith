use crate::diagnostics::{ConnectionState, DiagnosticStore, ScheduleHandling, TelegramRecord};
use crate::protocol::ProtocolError;
use crate::web::WebError;
use crate::*;
use crate::external::{self, ExternalInputMessage, ExternalInputUpdate};
use crate::{HostHealth, HostLimits, PendingWrites};
use logiksmith_core::{
    BlockId, BudgetProbe, BudgetProbeHandle, ClockSample, InputEvent, InputObservation,
    OutputEffect, Runtime as CoreRuntime, RuntimeProfile, RuntimeSimulationError, ScheduleName,
    ScheduleSimulationError, ScheduleSimulationRequest,
};
use std::{
    ffi::OsString,
    fmt, io,
    path::PathBuf,
    time::{Duration, Instant},
};
use thiserror::Error;
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    signal,
    sync::mpsc,
    time,
};
use tracing_subscriber::{
    EnvFilter, Layer, layer::SubscriberExt, util::SubscriberInitExt,
};

#[derive(Debug, Error)]
pub enum HostError {
    #[error("failed to start XKNX bridge `{path}`: {source}")]
    Start { path: PathBuf, source: io::Error },
    #[error("bridge protocol error: {0}")]
    Protocol(#[from] ProtocolError),
    #[error("bridge stdout reached EOF")]
    StdoutEof,
    #[error("KNX bridge fatal code={code}: {message}")]
    BridgeFatal { code: String, message: String },
    #[error("KNX bridge exited unexpectedly status={status}")]
    BridgeExited { status: String },
    #[error("runtime overload in {lane} queue (capacity={capacity}, depth={depth})")]
    Overload { lane: String, capacity: usize, depth: usize },
    #[error("pending KNX write timed out request_id={request_id}")]
    PendingWriteTimeout { request_id: u64 },
    #[error("pending KNX write capacity exhausted (capacity={capacity}, depth={depth})")]
    PendingWriteCapacity { capacity: usize, depth: usize },
    #[error("invalid runtime profile: {0}")]
    InvalidRuntimeProfile(String),
    #[error("fatal host condition: {0}")]
    FatalHost(String),
    #[error("bridge I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("bridge command result has unknown request_id={0}")]
    UnknownRequest(u64),
    #[error("dashboard startup failed: {0}")]
    Web(#[from] WebError),
}

pub async fn run(config: RuntimeConfig) -> Result<(), HostError> {
    run_with_bridge(
        config.clone(),
        BridgeCommand::new(
            config.bridge.python,
            vec![OsString::from("-m"), OsString::from("logiksmith_xknx")],
        ),
    )
    .await
}

/// Runs the browser editor and Lua simulator without spawning the KNX bridge.
/// Normal [`run`] startup keeps its fatal bridge-failure behaviour.
pub async fn run_simulation_only(config: RuntimeConfig) -> Result<(), HostError> {
    let limits = HostLimits::from_environment()
        .map_err(|error| HostError::InvalidRuntimeProfile(error.to_string()))?;
    let health = HostHealth::new(limits);
    let store = DiagnosticStore::new(
        &config.automation,
        config.automation_path.clone(),
        config.automation_revision,
    );
    store.set_host_limits(limits);
    init_logging(config.logging, store.clone());
    store.set_connection(ConnectionState::Disconnected);
    let mut runtime =
        CoreRuntime::try_new_with_limits(config.automation.core_config.clone(), limits.profile.limits()).map_err(|error| {
            HostError::Protocol(ProtocolError::Field("automation", error.to_string()))
        })?;
    let (activation_sender, activation_receiver) = mpsc::channel(limits.activation_queue);
    let (simulation_sender, simulation_receiver) = mpsc::channel(limits.simulation_queue);
    let (external_sender, external_receiver) = mpsc::channel(limits.external_input_queue);
    let web_server = web::start_web_server_with_runtime_and_sources_with_host(
        store.clone(),
        config.web,
        activation_sender,
        simulation_sender,
        &config.automation,
        external_sender.clone(),
        limits,
        health.clone(),
    )
    .await?;
    let external_tasks = external::spawn_http_polls(&config.automation, external_sender, store.clone(), limits, health.clone());
    tracing::info!(target: "logiksmith", "simulation-only mode ready; KNX bridge disabled");
    let result = run_simulation_session(
        &config,
        &store,
        &mut runtime,
        activation_receiver,
        simulation_receiver,
        external_receiver,
        limits,
        health.clone(),
    )
    .await;
    external_tasks.shutdown().await;
    web_server.shutdown().await;
    result
}

pub async fn run_with_bridge(
    config: RuntimeConfig,
    bridge_command: BridgeCommand,
) -> Result<(), HostError> {
    let limits = HostLimits::from_environment()
        .map_err(|error| HostError::InvalidRuntimeProfile(error.to_string()))?;
    let health = HostHealth::new(limits);
    let store = DiagnosticStore::new(
        &config.automation,
        config.automation_path.clone(),
        config.automation_revision,
    );
    store.set_host_limits(limits);
    init_logging(config.logging, store.clone());
    let mut runtime =
        CoreRuntime::try_new_with_limits(config.automation.core_config.clone(), limits.profile.limits()).map_err(|error| {
            HostError::Protocol(ProtocolError::Field("automation", error.to_string()))
        })?;
    let (activation_sender, activation_receiver) = mpsc::channel(limits.activation_queue);
    let (simulation_sender, simulation_receiver) = mpsc::channel(limits.simulation_queue);
    let (external_sender, external_receiver) = mpsc::channel(limits.external_input_queue);
    let web_server = web::start_web_server_with_runtime_and_sources_with_host(
        store.clone(),
        config.web,
        activation_sender,
        simulation_sender,
        &config.automation,
        external_sender.clone(),
        limits,
        health.clone(),
    )
    .await?;
    let external_tasks = external::spawn_http_polls(&config.automation, external_sender, store.clone(), limits, health.clone());
    let mut child = match Command::new(&bridge_command.executable)
        .args(&bridge_command.args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(source) => {
            external_tasks.shutdown().await;
            web_server.shutdown().await;
            return Err(HostError::Start {
                path: bridge_command.executable,
                source,
            });
        }
    };
    let mut stdin = child.stdin.take().ok_or_else(|| HostError::Start {
        path: bridge_command.executable.clone(),
        source: io::Error::other("bridge stdin was not piped"),
    })?;
    let stdout = child.stdout.take().ok_or_else(|| HostError::Start {
        path: bridge_command.executable.clone(),
        source: io::Error::other("bridge stdout was not piped"),
    })?;
    if let Some(stderr) = child.stderr.take() {
        tokio::spawn(forward_bridge_stderr(stderr));
    }
    let ingress = spawn_bridge_ingress(stdout, limits, store.clone(), health.clone());
    let result = run_session(
        &config,
        &store,
        &mut runtime,
        &mut child,
        &mut stdin,
        ingress,
        activation_receiver,
        simulation_receiver,
        external_receiver,
        limits,
        health.clone(),
    )
    .await;
    if let Err(error) = &result {
        store.record_runtime_fatal(error.to_string(), matches!(error, HostError::Overload { .. } | HostError::PendingWriteCapacity { .. }));
        if let HostError::PendingWriteTimeout { request_id } = error {
            store.record_pending_write_timeout(*request_id);
        }
        health.fail(error.to_string());
        let _ = send_message(&mut stdin, &shutdown_message()).await;
        terminate_child(&mut child).await;
    }
    external_tasks.shutdown().await;
    web_server.shutdown().await;
    result
}

async fn forward_bridge_stderr<R: AsyncRead + Unpin>(stderr: R) {
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        tracing::debug!(target: "bridge.xknx", "{line}");
    }
}

/// Messages read from the bridge are admitted through a bounded lane before
/// the serial runtime processes them.  A full lane is a process-wide failure:
/// dropping KNX telegrams would make output behaviour non-deterministic.
enum BridgeIngress {
    Message(Message),
    Eof,
    Error(String),
}

/// Drop-scoped measurement for one serial event-loop turn.  Keeping this as
/// a guard means early `continue` paths (for example an unbound telegram)
/// still contribute to the host timing projection.
struct HostTurnProbe {
    store: DiagnosticStore,
    limits: HostLimits,
    started: Instant,
}

/// Elapsed-time probe shared by the core's instruction hook and cascade
/// checks for one host event-loop turn. Desktop leaves it disabled; the
/// embedded profile uses the probe to enforce the soft OpenKNX budgets.
struct ElapsedBudgetProbe {
    started: Instant,
}

impl BudgetProbe for ElapsedBudgetProbe {
    fn elapsed_ms(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }
}

pub(crate) fn live_budget_probe(limits: HostLimits) -> Option<BudgetProbeHandle> {
    (limits.profile == RuntimeProfile::EmbeddedBaseline).then(|| {
        std::sync::Arc::new(ElapsedBudgetProbe {
            started: Instant::now(),
        }) as BudgetProbeHandle
    })
}

impl HostTurnProbe {
    fn new(store: &DiagnosticStore, limits: HostLimits) -> Self {
        Self {
            store: store.clone(),
            limits,
            started: Instant::now(),
        }
    }
}

impl Drop for HostTurnProbe {
    fn drop(&mut self) {
        let duration_us = u64::try_from(self.started.elapsed().as_micros()).unwrap_or(u64::MAX);
        self.store.record_host_turn(duration_us, self.limits);
    }
}

fn spawn_bridge_ingress(
    stdout: ChildStdout,
    limits: HostLimits,
    store: DiagnosticStore,
    health: HostHealth,
) -> mpsc::Receiver<BridgeIngress> {
    let (sender, receiver) = mpsc::channel(limits.knx_ingress_queue);
    tokio::spawn(async move {
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        loop {
            line.clear();
            let bytes = match reader.read_line(&mut line).await {
                Ok(bytes) => bytes,
                Err(error) => {
                    store.record_runtime_fatal(format!("bridge stdout read failed: {error}"), false);
                    health.fail(format!("bridge stdout read failed: {error}"));
                    let _ = sender.try_send(BridgeIngress::Error(error.to_string()));
                    return;
                }
            };
            if bytes == 0 {
                store.record_runtime_fatal("bridge stdout reached EOF", false);
                health.fail("bridge stdout reached EOF");
                let _ = sender.try_send(BridgeIngress::Eof);
                return;
            }
            let message = match parse_message(line.trim_end_matches(['\r', '\n'])) {
                Ok(message) => message,
                Err(error) => {
                    store.record_runtime_fatal(error.to_string(), false);
                    health.fail(error.to_string());
                    let _ = sender.try_send(BridgeIngress::Error(error.to_string()));
                    return;
                }
            };
            match sender.try_send(BridgeIngress::Message(message)) {
                Ok(()) => {
                    if health.snapshot().ready {
                        let depth = limits.knx_ingress_queue.saturating_sub(sender.capacity());
                        store.record_queue_admitted("knx_ingress", depth);
                    }
                }
                Err(mpsc::error::TrySendError::Full(_)) => {
                    store.record_queue_rejected("knx_ingress", limits.knx_ingress_queue, true);
                    let reason = format!(
                        "runtime overload in KNX ingress queue (capacity={}, depth={})",
                        limits.knx_ingress_queue, limits.knx_ingress_queue
                    );
                    store.record_runtime_fatal(reason.clone(), true);
                    health.fail(reason);
                    return;
                }
                Err(mpsc::error::TrySendError::Closed(_)) => return,
            }
        }
    });
    receiver
}

/// Applies a desktop-owned external source delivery through the same serial
/// runtime owner used by KNX events.  HTTP and webhook tasks never touch the
/// core directly, and a single source fans out in block declaration order.
async fn apply_external_input(
    runtime: &mut CoreRuntime,
    store: &DiagnosticStore,
    config: &RuntimeConfig,
    request: ExternalInputMessage,
    budget_probe: Option<BudgetProbeHandle>,
    mut bridge: Option<(&mut ChildStdin, &mut u64, &mut PendingWrites)>,
) -> Result<(), HostError> {
    let bindings = match &request.kind {
        external::ExternalInputKind::HttpPoll { .. } => config
            .automation
            .http_to_inputs
            .get(&request.source),
        external::ExternalInputKind::Webhook { .. } => config
            .automation
            .webhook_to_inputs
            .get(&request.source),
    };
    let Some(bindings) = bindings else {
        if let Some(reply) = request.reply {
            let _ = reply.send(Err(format!("unknown external source {:?}", request.source)));
        }
        return Ok(());
    };
    let origin = match &request.kind {
        external::ExternalInputKind::HttpPoll { poll } => {
            Some(diagnostics::ExecutionOrigin::Http {
                poll: poll.clone(),
                value: request.source.clone(),
            })
        }
        external::ExternalInputKind::Webhook { source } => {
            Some(diagnostics::ExecutionOrigin::Webhook {
                source: source.clone(),
            })
        }
    };
    let mut first_error = None;
    for binding in bindings {
        let update = match request.update {
            ExternalInputUpdate::Observe(value) => logiksmith_core::InputUpdate::Observe(value),
            ExternalInputUpdate::Trigger(value) => logiksmith_core::InputUpdate::Trigger(value),
            ExternalInputUpdate::Invalidate => logiksmith_core::InputUpdate::Invalidate,
        };
        let sample = clock_sample(store);
        let started = Instant::now();
        let result = match budget_probe.clone() {
            Some(probe) => runtime.process_input_update_sampled_with_budget_probe(
                &binding.block_id,
                binding.endpoint.clone(),
                update,
                sample,
                probe,
            ),
            None => runtime.process_input_update_sampled(
                &binding.block_id,
                binding.endpoint.clone(),
                update,
                sample,
            ),
        };
        match result {
            Ok(executions) => {
                let duration_us = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
                if !executions.is_empty() {
                    record_and_dispatch_cascade_with_origin(
                        runtime,
                        store,
                        &config.automation,
                        executions,
                        clock_sample(store).monotonic_ms,
                        duration_us,
                        bridge.as_mut().map(|(stdin, next_request_id, pending)| {
                            (&mut **stdin, &mut **next_request_id, &mut **pending)
                        }),
                        None,
                        origin.clone(),
                    )
                    .await?;
                } else {
                    store.set_runtime_projection_from_runtime(runtime, clock_sample(store).monotonic_ms);
                }
            }
            Err(error) => {
                tracing::warn!(target: "logiksmith", source = %request.source, block = %binding.block_id, endpoint = %binding.endpoint, error = %error, "ignoring invalid external input");
                first_error.get_or_insert_with(|| error.to_string());
            }
        }
    }
    if let Some(reply) = request.reply {
        let _ = reply.send(match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        });
    }
    Ok(())
}

async fn run_simulation_session(
    config: &RuntimeConfig,
    store: &DiagnosticStore,
    runtime: &mut CoreRuntime,
    mut activations: mpsc::Receiver<ActivationRequest>,
    mut simulations: mpsc::Receiver<SimulationRequest>,
    mut external: mpsc::Receiver<ExternalInputMessage>,
    limits: HostLimits,
    health: HostHealth,
) -> Result<(), HostError> {
    let interrupt = signal::ctrl_c();
    tokio::pin!(interrupt);
    let mut fatal = health.subscribe_fatal();
    initialise_schedules(runtime, store, config);
    let mut timer_sleep = Box::pin(time::sleep(timer_wait(runtime, store, config)));
    loop {
        tokio::select! {
            Some(request) = activations.recv() => {
                let _turn = HostTurnProbe::new(store, limits);
                apply_activation(runtime, store, config, request);
                reset_timer_sleep(&mut timer_sleep, runtime, store, config);
            }
            Some(request) = simulations.recv() => {
                let _turn = HostTurnProbe::new(store, limits);
                apply_simulation(runtime, store, config, request);
                reset_timer_sleep(&mut timer_sleep, runtime, store, config);
            }
            Some(request) = external.recv() => {
                let _turn = HostTurnProbe::new(store, limits);
                apply_external_input(runtime, store, config, request, live_budget_probe(limits), None).await?;
                reset_timer_sleep(&mut timer_sleep, runtime, store, config);
            }
            changed = fatal.changed() => {
                if changed.is_ok() && let Some(reason) = fatal.borrow().clone() {
                    return Err(HostError::FatalHost(reason));
                }
            }
            _ = &mut timer_sleep => {
                let _turn = HostTurnProbe::new(store, limits);
                drain_due_timers(runtime, store, config, None, limits).await?;
                poll_and_process_schedules(runtime, store, config, None, limits).await?;
                reset_timer_sleep(&mut timer_sleep, runtime, store, config);
            }
            signal = &mut interrupt => {
                signal?;
                return Ok(());
            }
        }
    }
}

async fn run_session(
    config: &RuntimeConfig,
    store: &DiagnosticStore,
    runtime: &mut CoreRuntime,
    child: &mut Child,
    stdin: &mut ChildStdin,
    mut ingress: mpsc::Receiver<BridgeIngress>,
    mut activations: mpsc::Receiver<ActivationRequest>,
    mut simulations: mpsc::Receiver<SimulationRequest>,
    mut external: mpsc::Receiver<ExternalInputMessage>,
    limits: HostLimits,
    health: HostHealth,
) -> Result<(), HostError> {
    let hello = match ingress.recv().await {
        Some(BridgeIngress::Message(Message::BridgeHello(hello))) => hello,
        Some(BridgeIngress::Message(Message::Fatal(fatal))) => {
            return Err(HostError::BridgeFatal { code: fatal.code, message: fatal.message });
        }
        Some(BridgeIngress::Error(error)) => return Err(HostError::FatalHost(error)),
        Some(BridgeIngress::Eof) | None => return Err(HostError::StdoutEof),
        Some(BridgeIngress::Message(_)) => {
            return Err(ProtocolError::Field(
                "startup",
                "expected bridge_hello before configure".to_owned(),
            )
            .into());
        }
    };
    tracing::info!(target: "logiksmith", bridge = %hello.bridge, bridge_version = %hello.bridge_version, xknx_version = %hello.xknx_version, "bridge hello");
    store.set_connection(ConnectionState::Connecting);
    send_message(stdin, &configure_message(config)).await?;
    let ready = match ingress.recv().await {
        Some(BridgeIngress::Message(Message::Ready(ready))) => ready,
        Some(BridgeIngress::Message(Message::Fatal(fatal))) => {
            return Err(HostError::BridgeFatal { code: fatal.code, message: fatal.message });
        }
        Some(BridgeIngress::Error(error)) => return Err(HostError::FatalHost(error)),
        Some(BridgeIngress::Eof) | None => return Err(HostError::StdoutEof),
        Some(BridgeIngress::Message(_)) => {
            return Err(ProtocolError::Field(
                "startup",
                "expected ready after configure".to_owned(),
            )
            .into());
        }
    };
    store.set_connection(ConnectionState::Connected);
    health.mark_ready();
    tracing::info!(target: "logiksmith", gateway = %ready.gateway, "KNX connected");
    let interrupt = signal::ctrl_c();
    tokio::pin!(interrupt);
    let mut next_request_id = 1u64;
    let mut pending = PendingWrites::new(limits.pending_knx_writes);
    let mut pending_deadlines = time::interval(Duration::from_millis(100));
    pending_deadlines.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
    let mut fatal = health.subscribe_fatal();
    initialise_schedules(runtime, store, config);
    let mut timer_sleep = Box::pin(time::sleep(timer_wait(runtime, store, config)));
    loop {
        tokio::select! {
            Some(incoming) = ingress.recv() => {
                let _turn = HostTurnProbe::new(store, limits);
                let message = match incoming {
                    BridgeIngress::Message(message) => message,
                    BridgeIngress::Eof => {
                        let status = child.wait().await?;
                        return Err(HostError::BridgeExited { status: format_status(status) });
                    }
                    BridgeIngress::Error(error) => return Err(HostError::FatalHost(error)),
                };
                match message {
                    Message::KnxEvent(event) => {
                        let destination = event.destination_address();
                        let logical_endpoint = config.automation.address_to_inputs.get(&destination).and_then(|bindings| bindings.first()).map(|binding| binding.endpoint.clone());
                        store.record_telegram(TelegramRecord::from_event(&event, logical_endpoint.as_ref()));
                        let Some(bindings) = config.automation.address_to_inputs.get(&destination) else { continue };
                        if event.value.is_none() { continue; }
                        let sample = clock_sample(store);
                        let now = sample.monotonic_ms;
                        if event.service == "group_value_write" {
                            let input_value = match event.typed_value()? {
                                Some(value) => value,
                                None => continue,
                            };
                            for binding in bindings {
                                let input = InputEvent::new(binding.endpoint.clone(), input_value);
                                let started = Instant::now();
                                let result = match live_budget_probe(limits) {
                                    Some(probe) => runtime.process_input_cascade_sampled_with_budget_probe(
                                        &binding.block_id,
                                        input,
                                        sample.clone(),
                                        probe,
                                    ),
                                    None => runtime.process_input_cascade_sampled(
                                        &binding.block_id,
                                        input,
                                        sample.clone(),
                                    ),
                                };
                                let duration_us = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
                                match result {
                                    Ok(executions) => record_and_dispatch_cascade_with_origin(
                                        runtime,
                                        store,
                                        &config.automation,
                                        executions,
                                        now,
                                        duration_us,
                                        Some((&mut *stdin, &mut next_request_id, &mut pending)),
                                        None,
                                        Some(diagnostics::ExecutionOrigin::Knx {
                                            group_address: Some(destination.to_string()),
                                        }),
                                    ).await?,
                                    Err(error) => tracing::warn!(target: "logiksmith", block = %binding.block_id, error = %error, "ignoring invalid logical input event"),
                                }
                            }
                        } else if event.service == "group_value_response"
                            && let Some(value) = event.typed_value()? {
                            for binding in bindings {
                                let observation = InputObservation::new(binding.endpoint.clone(), value);
                                if let Err(error) = runtime.observe_input(&binding.block_id, observation, now) {
                                    tracing::warn!(target: "logiksmith", block = %binding.block_id, error = %error, "ignoring invalid passive input observation");
                                }
                            }
                            store.set_runtime_projection_from_runtime(runtime, now);
                        }
                        reset_timer_sleep(&mut timer_sleep, runtime, store, config);
                    }
                    Message::CommandResult(result) => {
                        if !pending.remove(result.request_id) { return Err(HostError::UnknownRequest(result.request_id)); }
                        store.set_pending_knx_writes(pending.len());
                        store.record_write_result(result.request_id, result.ok, result.error.clone());
                        reset_timer_sleep(&mut timer_sleep, runtime, store, config);
                    }
                    Message::Fatal(fatal) => return Err(HostError::BridgeFatal { code: fatal.code, message: fatal.message }),
                    _ => return Err(ProtocolError::Field("runtime", "unexpected bridge message".to_owned()).into()),
                }
            }
            Some(request) = activations.recv() => {
                let _turn = HostTurnProbe::new(store, limits);
                apply_activation(runtime, store, config, request);
                reset_timer_sleep(&mut timer_sleep, runtime, store, config);
            }
            Some(request) = simulations.recv() => {
                let _turn = HostTurnProbe::new(store, limits);
                apply_simulation(runtime, store, config, request);
                reset_timer_sleep(&mut timer_sleep, runtime, store, config);
            }
            Some(request) = external.recv() => {
                let _turn = HostTurnProbe::new(store, limits);
                apply_external_input(runtime, store, config, request, live_budget_probe(limits), Some((&mut *stdin, &mut next_request_id, &mut pending))).await?;
                reset_timer_sleep(&mut timer_sleep, runtime, store, config);
            }
            _ = pending_deadlines.tick() => {
                let _turn = HostTurnProbe::new(store, limits);
                if let Some(request_id) = pending.expired(Instant::now(), Duration::from_millis(limits.pending_write_timeout_ms)) {
                    return Err(HostError::PendingWriteTimeout { request_id });
                }
            }
            changed = fatal.changed() => {
                if changed.is_ok() && let Some(reason) = fatal.borrow().clone() {
                    return Err(HostError::FatalHost(reason));
                }
            }
            _ = &mut timer_sleep => {
                let _turn = HostTurnProbe::new(store, limits);
                drain_due_timers(runtime, store, config, Some((&mut *stdin, &mut next_request_id, &mut pending)), limits).await?;
                poll_and_process_schedules(runtime, store, config, Some((&mut *stdin, &mut next_request_id, &mut pending)), limits).await?;
                reset_timer_sleep(&mut timer_sleep, runtime, store, config);
            }
            signal = &mut interrupt => {
                signal?;
                let _ = send_message(stdin, &shutdown_message()).await;
                terminate_child(child).await;
                return Ok(());
            }
        }
    }
}
