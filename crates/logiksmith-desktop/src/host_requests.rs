fn apply_activation(
    runtime: &mut CoreRuntime,
    store: &DiagnosticStore,
    config: &RuntimeConfig,
    request: ActivationRequest,
) {
    let ActivationRequest {
        updates,
        document_revision,
        document,
        reply,
    } = request;
    let result = runtime
        .activate(logiksmith_core::RuntimeActivation::new(updates))
        .map(|activation| {
            // Re-enable is a scheduler lifecycle boundary. Establish a
            // future-only baseline using one fresh paired sample so an
            // occurrence missed while disabled is never delivered on wake.
            for block in activation
                .blocks
                .iter()
                .filter(|block| block.enabled_changed && block.enabled)
            {
                let sample = clock_sample(store);
                if let Err(error) = runtime.rebaseline_block_schedules(&block.block_id, sample) {
                    tracing::warn!(
                        target: "logiksmith",
                        block = %block.block_id,
                        error = %error,
                        "schedule rebaseline after block re-enable failed"
                    );
                }
            }
            store.record_runtime_activation(
                document_revision,
                &document,
                &activation,
                runtime,
                &config.automation,
            );
            CoreActivationResult {
                document_revision,
                result: activation,
            }
        })
        .map_err(|error| error.to_string());
    let _ = reply.send(result);
}

pub(crate) fn apply_simulation(
    runtime: &CoreRuntime,
    store: &DiagnosticStore,
    config: &RuntimeConfig,
    request: SimulationRequest,
) {
    let SimulationRequest { payload, reply } = request;
    let Ok(block_id) = payload.block_id.parse::<BlockId>() else {
        let _ = reply.send(SimulationOutcome::NotFound);
        return;
    };
    let Some(block) = config.automation.block(&block_id) else {
        let _ = reply.send(SimulationOutcome::NotFound);
        return;
    };
    let Some(core_block) = runtime.block(&block_id) else {
        let _ = reply.send(SimulationOutcome::NotFound);
        return;
    };
    // The browser sends the persisted per-block revision. The core uses a
    // separate source-derived hash for timer ownership and stale execution
    // checks; never compare or expose that hash as the public revision.
    let active_document_revision = store
        .active_block_revision(&payload.block_id)
        .unwrap_or(block.revision);
    let active_core_revision = core_block.active_logic_revision();
    let outcome = if payload.expected_logic_revision != active_document_revision {
        if payload.trigger.schedule.is_some() {
            SimulationOutcome::ScheduleConflict {
                current_revision: active_document_revision,
                current_structural_revision: config.automation.structural_revision,
            }
        } else {
            SimulationOutcome::Conflict {
                current_revision: active_document_revision,
            }
        }
    } else {
        let started = Instant::now();
        let snapshot = core_block.snapshot_at(store.now());
        if payload.trigger.trigger_type.as_deref() == Some("timer") {
            match simulation_timer_scenario(
                &payload,
                block,
                active_document_revision,
                active_core_revision,
                Some(&snapshot.state),
            ) {
                Err(errors) => SimulationOutcome::Invalid(errors),
                Ok(scenario) => match runtime.simulate_timer(&block_id, scenario) {
                    Ok(execution) => {
                        SimulationOutcome::Complete(diagnostics::simulation_response_for_block(
                            &execution,
                            u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX),
                            active_document_revision,
                            &config.automation,
                        ))
                    }
                    Err(RuntimeSimulationError::Block { error, .. }) => {
                        SimulationOutcome::Invalid(simulation_error_fields(&error, &payload))
                    }
                    Err(RuntimeSimulationError::UnknownBlock(_)) => SimulationOutcome::NotFound,
                },
            }
        } else if payload.trigger.schedule.is_some() {
            schedule_simulation_outcome(
                runtime,
                store,
                config,
                &payload,
                &block_id,
                active_document_revision,
                active_core_revision,
            )
        } else {
            match simulation_scenario(payload.clone(), block) {
                Err(errors) => SimulationOutcome::Invalid(errors),
                Ok(scenario) => match simulation_state(&payload, Some(&snapshot.state)) {
                    Err(errors) => SimulationOutcome::Invalid(errors),
                    Ok(state) => match simulation_pending_timers(
                        &payload,
                        Some(&snapshot.pending_timers),
                        active_document_revision,
                        active_core_revision,
                    ) {
                        Err(errors) => SimulationOutcome::Invalid(errors),
                        Ok(pending_timers) => match runtime.simulate_input_with_state(
                            &block_id,
                            scenario,
                            state,
                            pending_timers,
                            store.now(),
                        ) {
                            Ok(execution) => SimulationOutcome::Complete(
                                diagnostics::simulation_response_for_block(
                                    &execution,
                                    u64::try_from(started.elapsed().as_micros())
                                        .unwrap_or(u64::MAX),
                                    active_document_revision,
                                    &config.automation,
                                ),
                            ),
                            Err(RuntimeSimulationError::Block { error, .. }) => {
                                SimulationOutcome::Invalid(simulation_error_fields(
                                    &error, &payload,
                                ))
                            }
                            Err(RuntimeSimulationError::UnknownBlock(_)) => {
                                SimulationOutcome::NotFound
                            }
                        },
                    },
                },
            }
        }
    };
    let _ = reply.send(outcome);
}

/// Handles a schedule trigger simulation. Without a selected instant the
/// outcome is a read-only occurrence preview; with one, the core builds the
/// exact trigger and time context a live occurrence would receive and runs
/// the same evaluator path without mutating live scheduler state.
fn schedule_simulation_outcome(
    runtime: &CoreRuntime,
    store: &DiagnosticStore,
    config: &RuntimeConfig,
    payload: &SimulationPayload,
    block_id: &BlockId,
    active_document_revision: u64,
    active_core_revision: u64,
) -> SimulationOutcome {
    let Some(name) = payload.trigger.schedule.as_deref() else {
        return SimulationOutcome::Invalid(vec![FieldError {
            path: "trigger.schedule".to_owned(),
            message: "is required for a schedule simulation".to_owned(),
        }]);
    };
    let schedule = match ScheduleName::new(name) {
        Ok(schedule) => schedule,
        Err(error) => {
            return SimulationOutcome::Invalid(vec![FieldError {
                path: "trigger.schedule".to_owned(),
                message: error.to_string(),
            }]);
        }
    };
    let active_structural_revision = config.automation.structural_revision;
    if let Some(expected) = payload.expected_structural_revision
        && expected != active_structural_revision
    {
        return SimulationOutcome::ScheduleConflict {
            current_revision: active_document_revision,
            current_structural_revision: active_structural_revision,
        };
    }
    let started = Instant::now();
    let Some(occurrence_at_ms) = payload.trigger.occurrence_at_ms else {
        // Preview: return the next occurrences without selecting one. The
        // wall clock anchors the search; an invalid clock cannot meaningfully
        // preview "next", so the request is rejected instead.
        let Some(after_utc_ms) = payload
            .preview_after_utc_ms
            .or_else(|| clock_sample(store).utc_unix_ms)
        else {
            return SimulationOutcome::Invalid(vec![FieldError {
                path: "trigger.occurrence_at_ms".to_owned(),
                message: "wall clock unavailable; cannot preview future occurrences".to_owned(),
            }]);
        };
        let count = payload.preview_count.unwrap_or(3).clamp(1, 10);
        let occurrences =
            match runtime.preview_occurrences(block_id, &schedule, after_utc_ms, count) {
                Ok(occurrences) => occurrences,
                Err(error) => {
                    if matches!(error, logiksmith_core::ScheduleError::UnknownSchedule) {
                        return SimulationOutcome::ScheduleNotFound;
                    }
                    return SimulationOutcome::Invalid(vec![FieldError {
                        path: "trigger.schedule".to_owned(),
                        message: error.to_string(),
                    }]);
                }
            };
        let document_schedule = config
            .automation
            .document
            .blocks
            .iter()
            .find(|block| block.id == payload.block_id)
            .and_then(|block| {
                block
                    .schedules
                    .iter()
                    .find(|candidate| candidate.name == name)
            });
        let Some(document_schedule) = document_schedule else {
            return SimulationOutcome::ScheduleNotFound;
        };
        return SimulationOutcome::Previews(diagnostics::schedule_preview_response(
            &payload.block_id,
            document_schedule,
            &occurrences,
            config.automation.core_config.site.timezone.as_str(),
        ));
    };
    let request = ScheduleSimulationRequest {
        block_id: block_id.clone(),
        expected_logic_revision: active_core_revision,
        expected_structural_revision: active_structural_revision,
        schedule,
        occurrence_at_utc_ms: occurrence_at_ms,
    };
    match runtime.simulate_schedule(request) {
        Ok(execution) => SimulationOutcome::Complete(diagnostics::simulation_response_for_block(
            &execution,
            u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX),
            active_document_revision,
            &config.automation,
        )),
        Err(ScheduleSimulationError::StaleStructuralRevision) => {
            SimulationOutcome::ScheduleConflict {
                current_revision: active_document_revision,
                current_structural_revision: active_structural_revision,
            }
        }
        Err(ScheduleSimulationError::UnknownSchedule) => SimulationOutcome::ScheduleNotFound,
        Err(ScheduleSimulationError::NotOccurrence) => {
            SimulationOutcome::Invalid(vec![FieldError {
                path: "trigger.occurrence_at_ms".to_owned(),
                message: "selected instant is not a valid occurrence of this schedule".to_owned(),
            }])
        }
        Err(ScheduleSimulationError::InvalidInput(error)) => {
            SimulationOutcome::Invalid(simulation_error_fields(&error, payload))
        }
    }
}
