fn snapshot_locked(inner: &Inner, now: logiksmith_core::MonotonicMs) -> Snapshot {
    let values = ValuesSnapshot {
        endpoints: inner
            .endpoint_values
            .iter()
            .map(|(name, state)| EndpointValueSnapshot {
                name: name.to_string(),
                direction: state.direction.to_string(),
                dpt: DptMessage::from_core(state.dpt),
                observed: state.observed.clone(),
                requested: state.requested.clone(),
            })
            .collect(),
    };
    let blocks = inner
        .block_order
        .iter()
        .filter_map(|id| inner.blocks.get(id).map(|state| (id, state)))
        .map(|(id, state)| {
            let automation = inner.block_automation.get(id);
            BlockSnapshot {
                id: id.clone(),
                active_enabled: state.active_enabled,
                saved_enabled: state.saved_enabled,
                active_revision: state.active_logic_revision,
                saved_revision: state.saved_logic_revision,
                active_logic_revision: state.active_logic_revision,
                saved_logic_revision: state.saved_logic_revision,
                source: if state.source.is_empty() {
                    automation
                        .map(|automation| automation.logic.source.clone())
                        .unwrap_or_default()
                } else {
                    state.source.clone()
                },
                inputs: automation
                    .map(|automation| automation.inputs.clone())
                    .unwrap_or_default(),
                outputs: automation
                    .map(|automation| automation.outputs.clone())
                    .unwrap_or_default(),
                knx_bindings: automation
                    .map(|automation| automation.knx_bindings.clone())
                    .unwrap_or_default(),
                signal_bindings: automation
                    .map(|automation| automation.signal_bindings.clone())
                    .unwrap_or_default(),
                http_bindings: automation
                    .map(|automation| automation.http_bindings.clone())
                    .unwrap_or_default(),
                webhook_bindings: automation
                    .map(|automation| automation.webhook_bindings.clone())
                    .unwrap_or_default(),
                values: ValuesSnapshot {
                    endpoints: inner
                        .block_endpoint_values
                        .iter()
                        .filter(|((block_id, _), _)| block_id == id)
                        .map(|((_, name), state)| EndpointValueSnapshot {
                            name: name.to_string(),
                            direction: state.direction.to_string(),
                            dpt: DptMessage::from_core(state.dpt),
                            observed: state.observed.clone(),
                            requested: state.requested.clone(),
                        })
                        .collect(),
                },
                state: state.state.clone(),
                pending_timers: state.pending_timers.clone(),
                executions: state.executions.iter().rev().cloned().collect(),
                schedules: block_schedule_snapshots(inner, id, state),
                last_result: state.last_result.clone(),
            }
        })
        .collect();
    let mut external_inputs = inner.external_inputs.clone();
    for poll in &mut external_inputs.http_polls {
        for value in &mut poll.values {
            value.age_ms = value
                .observed_at_ms
                .map(|observed| now.0.saturating_sub(observed));
        }
    }
    for webhook in &mut external_inputs.webhook_inputs {
        webhook.age_ms = webhook
            .observed_at_ms
            .map(|observed| now.0.saturating_sub(observed));
    }
    Snapshot {
        revision: inner.revision,
        connection: ConnectionSnapshot {
            state: inner.connection,
        },
        config: ConfigSnapshot {
            active: inner.automation.clone(),
        },
        automation: inner.automation.clone(),
        active_automation_revision: inner.active_automation_revision,
        saved_automation_revision: inner.saved_automation_revision,
        captured_at_ms: inner.captured_at_ms,
        site_time: inner.site_time.clone(),
        state: inner.state.clone(),
        pending_timers: inner.pending_timers.clone(),
        values,
        write: inner.last_write.clone(),
        logic: LogicStatusSnapshot {
            active_logic_revision: inner.active_logic_revision,
            saved_logic_revision: inner.saved_logic_revision,
            active_structural_revision: inner.active_structural_revision,
            saved_structural_revision: inner.saved_structural_revision,
            restart_required: inner.restart_required,
            state: inner.state.clone(),
            pending_timers: inner.pending_timers.clone(),
            executions: inner.executions.iter().rev().cloned().collect(),
        },
        telegrams: inner.telegrams.iter().cloned().collect(),
        logs: inner.logs.iter().cloned().collect(),
        blocks,
        signals: inner.signals.clone(),
        external_inputs,
    }
}
