fn timer_wait(runtime: &CoreRuntime, store: &DiagnosticStore, config: &RuntimeConfig) -> Duration {
    let timer_wait = runtime
        .next_timer_deadline()
        .map(|deadline| Duration::from_millis(deadline.0.saturating_sub(store.now().0)))
        .unwrap_or(Duration::from_secs(86_400));
    // Fold the earliest UTC schedule deadline into the same monotonic sleep.
    // The wall clock is sampled fresh so the schedule deadline can be mapped
    // onto the host monotonic clock; the sleep is capped at 30 seconds so the
    // host resamples the wall clock periodically even when the next schedule
    // occurrence is far away.
    let sample = clock_sample(store);
    let schedule_wait = match (runtime.next_schedule_deadline(), sample.utc_unix_ms) {
        (Some(deadline), Some(wall_now_ms)) => {
            Duration::from_millis(deadline.0.saturating_sub(wall_now_ms).min(30_000) as u64)
        }
        _ if config
            .automation
            .blocks
            .iter()
            .any(|block| !block.schedules.is_empty()) =>
        {
            Duration::from_secs(30)
        }
        _ => Duration::from_secs(86_400),
    };
    timer_wait.min(schedule_wait)
}

fn reset_timer_sleep(
    sleep: &mut std::pin::Pin<Box<time::Sleep>>,
    runtime: &CoreRuntime,
    store: &DiagnosticStore,
    config: &RuntimeConfig,
) {
    sleep
        .as_mut()
        .reset(time::Instant::now() + timer_wait(runtime, store, config));
}

async fn drain_due_timers(
    runtime: &mut CoreRuntime,
    store: &DiagnosticStore,
    config: &RuntimeConfig,
    mut bridge: Option<(&mut ChildStdin, &mut u64, &mut HashSet<u64>)>,
) -> Result<(), HostError> {
    let now = store.now();
    while runtime
        .next_timer_deadline()
        .is_some_and(|deadline| deadline <= now)
    {
        let started = Instant::now();
        let sample = clock_sample(store);
        let executions = match runtime.process_next_due_timer_cascade_sampled(sample) {
            Ok(executions) if executions.is_empty() => break,
            Ok(executions) => executions,
            Err(error) => {
                tracing::warn!(target: "logiksmith", error = %error, "discarding invalid timer execution");
                store.set_runtime_projection_from_runtime(runtime, now);
                continue;
            }
        };
        let duration_us = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
        record_and_dispatch_cascade(
            runtime,
            store,
            &config.automation,
            executions,
            now,
            duration_us,
            bridge.as_mut().map(|(stdin, next_request_id, pending)| {
                (&mut **stdin, &mut **next_request_id, &mut **pending)
            }),
            None,
        )
        .await?;
    }
    Ok(())
}

async fn record_and_dispatch_cascade(
    runtime: &CoreRuntime,
    store: &DiagnosticStore,
    automation: &AutomationRuntime,
    executions: Vec<logiksmith_core::BlockExecution>,
    now: logiksmith_core::MonotonicMs,
    duration_us: u64,
    bridge: Option<(&mut ChildStdin, &mut u64, &mut HashSet<u64>)>,
    schedule_handling: Option<ScheduleHandling>,
) -> Result<(), HostError> {
    record_and_dispatch_cascade_with_origin(
        runtime,
        store,
        automation,
        executions,
        now,
        duration_us,
        bridge,
        schedule_handling,
        None,
    )
    .await
}

async fn record_and_dispatch_cascade_with_origin(
    runtime: &CoreRuntime,
    store: &DiagnosticStore,
    automation: &AutomationRuntime,
    executions: Vec<logiksmith_core::BlockExecution>,
    now: logiksmith_core::MonotonicMs,
    duration_us: u64,
    mut bridge: Option<(&mut ChildStdin, &mut u64, &mut HashSet<u64>)>,
    schedule_handling: Option<ScheduleHandling>,
    origin: Option<diagnostics::ExecutionOrigin>,
) -> Result<(), HostError> {
    for execution in executions {
        store.record_block_execution_with_origin(
            &execution,
            now,
            duration_us,
            automation,
            schedule_handling,
            origin.clone(),
        );
        if let Some((stdin, next_request_id, pending)) = bridge.as_mut()
            && let Ok(transition) = &execution.execution.outcome
        {
            dispatch_effects(
                store,
                stdin,
                automation,
                &execution.block_id,
                transition.outputs.clone(),
                next_request_id,
                pending,
            )
            .await?;
        }
    }
    store.set_runtime_projection_from_runtime(runtime, now);
    Ok(())
}

/// Current UTC wall clock in milliseconds, or `None` when the host clock is
/// unavailable or cannot represent the instant.
fn wall_clock_ms() -> Option<i64> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
}

/// Pairs the host wall clock with the store monotonic clock into one sample
/// for the portable schedule engine. The wall clock is optional: `None` means
/// the host clock is unavailable and wall-clock schedules pause.
fn clock_sample(store: &DiagnosticStore) -> ClockSample {
    ClockSample {
        monotonic_ms: store.now(),
        utc_unix_ms: wall_clock_ms(),
    }
}

/// Establishes the schedule baseline at startup and publishes the first live
/// site card and per-schedule status feed. Failures are logged and leave
/// wall-clock schedules paused; input and timer handling stay alive.
fn initialise_schedules(
    runtime: &mut CoreRuntime,
    store: &DiagnosticStore,
    config: &RuntimeConfig,
) {
    let sample = clock_sample(store);
    let utc_unix_ms = sample.utc_unix_ms;
    let site_time =
        diagnostics::site_time_snapshot_live(&config.automation.core_config.site, &sample);
    match runtime.initialise_schedules(sample, config.automation.structural_revision) {
        Ok(()) => {
            store.set_site_time_sample(sample, site_time);
            refresh_schedule_statuses(runtime, store, utc_unix_ms);
        }
        Err(error) => {
            tracing::warn!(target: "logiksmith", error = %error, "schedule initialisation failed; wall-clock schedules paused");
            store.set_site_time_sample(sample, site_time);
            refresh_schedule_statuses(runtime, store, utc_unix_ms);
        }
    }
}

/// Refreshes the per-schedule status feed from the core scheduler after a
/// poll, restart, or clock sample.
fn refresh_schedule_statuses(
    runtime: &CoreRuntime,
    store: &DiagnosticStore,
    utc_unix_ms: Option<i64>,
) {
    let statuses: Vec<_> = runtime
        .schedule_statuses(utc_unix_ms)
        .iter()
        .map(diagnostics::schedule_status_feed)
        .collect();
    store.set_schedule_statuses(statuses);
}

/// Polls the schedule engine with a fresh paired sample, routes each due
/// trigger through the per-block serial execution path, records the execution
/// with host-measured handling facts, and dispatches output effects through
/// the existing bridge path. Also refreshes the site card and per-schedule
/// status feed so the dashboard reflects the latest scheduler state.
async fn poll_and_process_schedules(
    runtime: &mut CoreRuntime,
    store: &DiagnosticStore,
    config: &RuntimeConfig,
    mut bridge: Option<(&mut ChildStdin, &mut u64, &mut HashSet<u64>)>,
) -> Result<(), HostError> {
    let sample = clock_sample(store);
    let utc_unix_ms = sample.utc_unix_ms;
    store.set_site_time_sample(
        sample,
        diagnostics::site_time_snapshot_live(&config.automation.core_config.site, &sample),
    );
    let triggers = match runtime.poll_schedules(sample) {
        Ok(triggers) => triggers,
        Err(error) => {
            tracing::warn!(target: "logiksmith", error = %error, "schedule poll failed; wall-clock schedules paused");
            refresh_schedule_statuses(runtime, store, utc_unix_ms);
            return Ok(());
        }
    };
    for trigger in triggers {
        let block_id = trigger.block_id.clone();
        let coalesced_count = trigger.coalesced_count;
        let started = Instant::now();
        // Keep all schedule handling facts tied to the paired poll sample.
        // A second SystemTime read could cross a wall-clock correction and
        // make queue delay disagree with the scheduler's detection instant.
        let handled_at_utc_ms = utc_unix_ms.unwrap_or(0);
        let executions = match runtime.process_schedule_cascade_sampled(trigger, sample) {
            Ok(executions) if executions.is_empty() => {
                // Stale structural revision, disabled block, or unknown
                // schedule; the core decided not to deliver.
                continue;
            }
            Ok(executions) => executions,
            Err(error) => {
                tracing::warn!(target: "logiksmith", block = %block_id, error = %error, "discarding invalid schedule execution");
                continue;
            }
        };
        let duration_us = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
        let handling = diagnostics::ScheduleHandling {
            handled_at_utc_ms,
            coalesced_count,
        };
        record_and_dispatch_cascade(
            runtime,
            store,
            &config.automation,
            executions,
            store.now(),
            duration_us,
            bridge.as_mut().map(|(stdin, next_request_id, pending)| {
                (&mut **stdin, &mut **next_request_id, &mut **pending)
            }),
            Some(handling),
        )
        .await?;
    }
    refresh_schedule_statuses(runtime, store, utc_unix_ms);
    Ok(())
}
