use crate::diagnostics::{ConnectionState, DiagnosticStore, ScheduleHandling, TelegramRecord};
use crate::protocol::ProtocolError;
use crate::web::WebError;
use crate::*;
use logiksmith_core::{
    BlockId, ClockSample, InputEvent, InputObservation, OutputEffect, Runtime as CoreRuntime,
    RuntimeSimulationError, ScheduleName, ScheduleSimulationError,
    ScheduleSimulationRequest,
};
use std::{
    collections::HashSet,
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
    let store = DiagnosticStore::new(
        &config.automation,
        config.automation_path.clone(),
        config.automation_revision,
    );
    init_logging(config.logging, store.clone());
    store.set_connection(ConnectionState::Disconnected);
    let mut runtime =
        CoreRuntime::try_new(config.automation.core_config.clone()).map_err(|error| {
            HostError::Protocol(ProtocolError::Field("automation", error.to_string()))
        })?;
    let (activation_sender, activation_receiver) = mpsc::channel(8);
    let (simulation_sender, simulation_receiver) = mpsc::channel(8);
    let web_server = web::start_web_server_with_runtime(
        store.clone(),
        config.web,
        activation_sender,
        simulation_sender,
    )
    .await?;
    tracing::info!(target: "logiksmith", "simulation-only mode ready; KNX bridge disabled");
    let result = run_simulation_session(
        &config,
        &store,
        &mut runtime,
        activation_receiver,
        simulation_receiver,
    )
    .await;
    web_server.shutdown().await;
    result
}

pub async fn run_with_bridge(
    config: RuntimeConfig,
    bridge_command: BridgeCommand,
) -> Result<(), HostError> {
    let store = DiagnosticStore::new(
        &config.automation,
        config.automation_path.clone(),
        config.automation_revision,
    );
    init_logging(config.logging, store.clone());
    let mut runtime =
        CoreRuntime::try_new(config.automation.core_config.clone()).map_err(|error| {
            HostError::Protocol(ProtocolError::Field("automation", error.to_string()))
        })?;
    let (activation_sender, activation_receiver) = mpsc::channel(8);
    let (simulation_sender, simulation_receiver) = mpsc::channel(8);
    let web_server = web::start_web_server_with_runtime(
        store.clone(),
        config.web,
        activation_sender,
        simulation_sender,
    )
    .await?;
    let mut child = match Command::new(&bridge_command.executable)
        .args(&bridge_command.args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(source) => {
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
    let mut reader = BufReader::new(stdout);
    let result = run_session(
        &config,
        &store,
        &mut runtime,
        &mut child,
        &mut stdin,
        &mut reader,
        activation_receiver,
        simulation_receiver,
    )
    .await;
    if result.is_err() {
        let _ = send_message(&mut stdin, &shutdown_message()).await;
        terminate_child(&mut child).await;
    }
    web_server.shutdown().await;
    result
}

async fn forward_bridge_stderr<R: AsyncRead + Unpin>(stderr: R) {
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        tracing::debug!(target: "bridge.xknx", "{line}");
    }
}

async fn run_simulation_session(
    config: &RuntimeConfig,
    store: &DiagnosticStore,
    runtime: &mut CoreRuntime,
    mut activations: mpsc::Receiver<ActivationRequest>,
    mut simulations: mpsc::Receiver<SimulationRequest>,
) -> Result<(), HostError> {
    let interrupt = signal::ctrl_c();
    tokio::pin!(interrupt);
    initialise_schedules(runtime, store, config);
    let mut timer_sleep = Box::pin(time::sleep(timer_wait(runtime, store, config)));
    loop {
        tokio::select! {
            Some(request) = activations.recv() => {
                apply_activation(runtime, store, config, request);
                reset_timer_sleep(&mut timer_sleep, runtime, store, config);
            }
            Some(request) = simulations.recv() => {
                apply_simulation(runtime, store, config, request);
                reset_timer_sleep(&mut timer_sleep, runtime, store, config);
            }
            _ = &mut timer_sleep => {
                drain_due_timers(runtime, store, config, None).await?;
                poll_and_process_schedules(runtime, store, config, None).await?;
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
    reader: &mut BufReader<ChildStdout>,
    mut activations: mpsc::Receiver<ActivationRequest>,
    mut simulations: mpsc::Receiver<SimulationRequest>,
) -> Result<(), HostError> {
    let hello = match read_message(reader).await? {
        Message::BridgeHello(hello) => hello,
        Message::Fatal(fatal) => {
            return Err(HostError::BridgeFatal {
                code: fatal.code,
                message: fatal.message,
            });
        }
        _ => {
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
    let ready = match read_message(reader).await? {
        Message::Ready(ready) => ready,
        Message::Fatal(fatal) => {
            return Err(HostError::BridgeFatal {
                code: fatal.code,
                message: fatal.message,
            });
        }
        _ => {
            return Err(ProtocolError::Field(
                "startup",
                "expected ready after configure".to_owned(),
            )
            .into());
        }
    };
    store.set_connection(ConnectionState::Connected);
    tracing::info!(target: "logiksmith", gateway = %ready.gateway, "KNX connected");
    let interrupt = signal::ctrl_c();
    tokio::pin!(interrupt);
    let mut line = String::new();
    let mut next_request_id = 1u64;
    let mut pending = HashSet::new();
    initialise_schedules(runtime, store, config);
    let mut timer_sleep = Box::pin(time::sleep(timer_wait(runtime, store, config)));
    loop {
        tokio::select! {
            read = reader.read_line(&mut line) => {
                let bytes = read?;
                if bytes == 0 { let status = child.wait().await?; return Err(HostError::BridgeExited { status: format_status(status) }); }
                let message = parse_message(line.trim_end_matches(['\r', '\n']))?;
                line.clear();
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
                                let result = runtime.process_input_cascade_sampled(&binding.block_id, input, sample.clone());
                                let duration_us = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
                                match result {
                                    Ok(executions) => record_and_dispatch_cascade(
                                        runtime,
                                        store,
                                        &config.automation,
                                        executions,
                                        now,
                                        duration_us,
                                        Some((&mut *stdin, &mut next_request_id, &mut pending)),
                                        None,
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
                        if !pending.remove(&result.request_id) { return Err(HostError::UnknownRequest(result.request_id)); }
                        store.record_write_result(result.request_id, result.ok, result.error.clone());
                        reset_timer_sleep(&mut timer_sleep, runtime, store, config);
                    }
                    Message::Fatal(fatal) => return Err(HostError::BridgeFatal { code: fatal.code, message: fatal.message }),
                    _ => return Err(ProtocolError::Field("runtime", "unexpected bridge message".to_owned()).into()),
                }
            }
            Some(request) = activations.recv() => {
                apply_activation(runtime, store, config, request);
                reset_timer_sleep(&mut timer_sleep, runtime, store, config);
            }
            Some(request) = simulations.recv() => {
                apply_simulation(runtime, store, config, request);
                reset_timer_sleep(&mut timer_sleep, runtime, store, config);
            }
            _ = &mut timer_sleep => {
                drain_due_timers(runtime, store, config, Some((&mut *stdin, &mut next_request_id, &mut pending))).await?;
                poll_and_process_schedules(runtime, store, config, Some((&mut *stdin, &mut next_request_id, &mut pending))).await?;
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
