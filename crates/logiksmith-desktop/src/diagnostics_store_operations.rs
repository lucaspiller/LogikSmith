fn initial_block_health(
    blocks: &BTreeMap<String, BlockDiagnosticState>,
) -> BTreeMap<String, BlockHealthSnapshot> {
    blocks
        .iter()
        .map(|(block_id, block)| {
            (
                block_id.clone(),
                BlockHealthSnapshot {
                    status: if block.active_enabled {
                        "active".to_owned()
                    } else {
                        "disabled".to_owned()
                    },
                    ..BlockHealthSnapshot::default()
                },
            )
        })
        .collect()
}

fn operations_snapshot(limits: crate::HostLimits) -> OperationsSnapshot {
    let queue = |capacity| QueueSnapshot {
        capacity,
        depth: 0,
        high_water: 0,
        accepted: 0,
        rejected: 0,
    };
    let mut queues = BTreeMap::new();
    queues.insert("knx_ingress".to_owned(), queue(limits.knx_ingress_queue));
    queues.insert("external_input".to_owned(), queue(limits.external_input_queue));
    queues.insert("activation".to_owned(), queue(limits.activation_queue));
    queues.insert("simulation".to_owned(), queue(limits.simulation_queue));
    OperationsSnapshot {
        profile: limits.profile.as_str().to_owned(),
        status: "healthy".to_owned(),
        queues,
        core: core_usage_snapshot(limits.profile, logiksmith_core::RuntimeUsage::default()),
        host_turn: HostTurnSnapshot::default(),
        block_health: BTreeMap::new(),
        pending_knx_writes: 0,
        pending_knx_write_capacity: limits.pending_knx_writes,
        pending_write_timeouts: 0,
        fatal: None,
    }
}

fn core_usage_snapshot(
    profile: logiksmith_core::RuntimeProfile,
    usage: logiksmith_core::RuntimeUsage,
) -> CoreUsageSnapshot {
    let limits = profile.limits();
    let counter = |used: usize, capacity: usize| CapacitySnapshot { used, capacity };
    CoreUsageSnapshot {
        logic_blocks: counter(usage.logic_blocks, limits.max_logic_blocks),
        signals: counter(usage.signals, limits.max_signals),
        signal_bindings: counter(usage.signal_bindings, limits.max_signal_bindings),
        logic_source_bytes: counter(usage.logic_source_bytes, limits.max_logic_source_bytes_total),
        state_entries: counter(
            usage.state_entries,
            limits
                .max_state_entries_per_block
                .saturating_mul(limits.max_logic_blocks),
        ),
        state_bytes: counter(usage.state_bytes, limits.max_state_bytes_total),
        pending_timers: counter(usage.pending_timers, limits.max_pending_timers_total),
    }
}
