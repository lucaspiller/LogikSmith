fn signal_consumer_record(
    endpoint: &logiksmith_core::SignalEndpointId,
) -> SignalConsumerSnapshot {
    SignalConsumerSnapshot {
        block_id: endpoint.block_id.to_string(),
        endpoint: endpoint.endpoint.to_string(),
    }
}

fn signal_effect_records(
    effects: &[logiksmith_core::SignalEffect],
) -> Vec<LogicalSignalEffectRecord> {
    effects
        .iter()
        .map(|effect| LogicalSignalEffectRecord {
            endpoint: effect.producer.endpoint.to_string(),
            signal: effect.signal.to_string(),
            dpt: DptMessage::from_core(effect.value.dpt),
            value: ValueMessage::from_core(effect.value),
            changed: effect.changed,
            producer: Some(SignalProducerSnapshot {
                block_id: effect.producer.block_id.to_string(),
                endpoint: effect.producer.endpoint.to_string(),
                execution_id: effect.producing_execution,
            }),
            producing_execution_id: effect.producing_execution,
            consumers: effect
                .consumers
                .iter()
                .map(signal_consumer_record)
                .collect(),
        })
        .collect()
}

fn signal_snapshots(runtime: &AutomationRuntime) -> Vec<SignalSnapshot> {
    runtime
        .signals
        .iter()
        .map(|signal| {
            let producer = runtime
                .output_to_signal
                .iter()
                .find(|(_, candidate)| candidate == &&signal.name)
                .map(|((block_id, endpoint), _)| SignalProducerSnapshot {
                    block_id: block_id.clone(),
                    endpoint: endpoint.to_string(),
                    execution_id: None,
                });
            let mut consumers = runtime
                .signal_to_inputs
                .get(&signal.name)
                .into_iter()
                .flatten()
                .map(|binding| SignalConsumerSnapshot {
                    block_id: binding.block_id.clone(),
                    endpoint: binding.endpoint.to_string(),
                })
                .collect::<Vec<_>>();
            consumers.sort_by(|left, right| {
                left.block_id
                    .cmp(&right.block_id)
                    .then(left.endpoint.cmp(&right.endpoint))
            });
            SignalSnapshot {
                name: signal.name.clone(),
                dpt: DptMessage::from_core(signal.dpt),
                value: None,
                status: "unknown".to_owned(),
                producer,
                consumers,
                observed_at_ms: None,
                changed_at_ms: None,
                producing_execution_id: None,
                recent_changes: Vec::new(),
                structural_revision: Some(runtime.structural_revision),
            }
        })
        .collect()
}

fn signal_snapshot_record(
    signal: &logiksmith_core::SignalSnapshot,
    previous: Option<&SignalSnapshot>,
    structural_revision: u64,
) -> SignalSnapshot {
    let producer = |endpoint: &logiksmith_core::SignalEndpointId| SignalProducerSnapshot {
        block_id: endpoint.block_id.to_string(),
        endpoint: endpoint.endpoint.to_string(),
        execution_id: signal.producing_execution,
    };
    let consumer = |endpoint: &logiksmith_core::SignalEndpointId| SignalConsumerSnapshot {
        block_id: endpoint.block_id.to_string(),
        endpoint: endpoint.endpoint.to_string(),
    };
    let status = match signal.status {
        logiksmith_core::SignalStatus::Unknown => "unknown",
        logiksmith_core::SignalStatus::Valid => "valid",
        logiksmith_core::SignalStatus::ProducerDisabled => "producer_disabled",
    };
    let value = signal.value.map(ValueMessage::from_core);
    let mut recent_changes = previous
        .map(|previous| previous.recent_changes.clone())
        .unwrap_or_default();
    if previous.is_none_or(|previous| previous.value != value) && value.is_some() {
        recent_changes.insert(
            0,
            SignalChangeSnapshot {
                value: value.clone(),
                observed_at_ms: signal.observed_at.map(|time| time.0),
                changed_at_ms: signal.changed_at.map(|time| time.0),
                execution_id: signal.producing_execution,
            },
        );
        recent_changes.truncate(MAX_EXECUTIONS);
    }
    SignalSnapshot {
        name: signal.name.to_string(),
        dpt: DptMessage::from_core(signal.dpt),
        value,
        status: status.to_owned(),
        producer: signal.producer.as_ref().map(producer),
        consumers: signal.consumers.iter().map(consumer).collect(),
        observed_at_ms: signal.observed_at.map(|time| time.0),
        changed_at_ms: signal.changed_at.map(|time| time.0),
        producing_execution_id: signal.producing_execution,
        recent_changes,
        structural_revision: Some(structural_revision),
    }
}
