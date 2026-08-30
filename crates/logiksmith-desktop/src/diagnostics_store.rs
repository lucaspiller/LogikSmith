fn unique_execution_id(inner: &Inner, requested: Option<u64>) -> u64 {
    let mut candidate = requested
        .unwrap_or(inner.next_execution_id)
        .max(inner.next_execution_id);
    while inner
        .executions
        .iter()
        .any(|record| record.execution_id == candidate)
        || inner
            .blocks
            .values()
            .any(|block| block.executions.iter().any(|record| record.execution_id == candidate))
    {
        candidate = candidate.saturating_add(1);
    }
    candidate
}

include!("diagnostics_store_operations.rs");

impl DiagnosticStore {
    pub fn new(runtime: &AutomationRuntime, automation_path: PathBuf, revision: u64) -> Self {
        let (events, _) = broadcast::channel(JOURNAL_CAPACITY);
        let mut endpoint_values = BTreeMap::new();
        let mut block_endpoint_values = BTreeMap::new();
        let mut blocks = BTreeMap::new();
        let mut block_automation = BTreeMap::new();
        let mut block_schedules = BTreeMap::new();
        for block in &runtime.blocks {
            let block_id = block.id.to_string();
            block_schedules.insert(
                block_id.clone(),
                runtime
                    .document
                    .blocks
                    .iter()
                    .find(|candidate| candidate.id == block.id.as_str())
                    .map(|candidate| {
                        candidate
                            .schedules
                            .iter()
                            .map(schedule_config_snapshot)
                            .collect()
                    })
                    .unwrap_or_default(),
            );
        }
        for block in &runtime.blocks {
            let block_id = block.id.to_string();
            let source = runtime
                .document
                .blocks
                .iter()
                .find(|candidate| candidate.id == block.id.as_str())
                .map(|candidate| candidate.source.clone())
                .unwrap_or_default();
            let revision = block.revision;
            blocks.insert(
                block_id.clone(),
                BlockDiagnosticState {
                    active_enabled: block.enabled,
                    saved_enabled: block.enabled,
                    active_logic_revision: revision,
                    saved_logic_revision: revision,
                    source,
                    state: BTreeMap::new(),
                    pending_timers: Vec::new(),
                    executions: VecDeque::new(),
                    last_result: None,
                },
            );
            block_automation.insert(block_id.clone(), block_automation_snapshot(runtime, block));
            for endpoint in block.engine_config.endpoints.iter() {
                let address = block.endpoint_to_address.get(&endpoint.name).copied();
                if address.is_none()
                    && !block.endpoint_to_signal.contains_key(&endpoint.name)
                    && !block.endpoint_to_external.contains_key(&endpoint.name)
                {
                    continue;
                }
                // The legacy global value projection is retained for the
                // desktop shell while block snapshots carry the authoritative
                // identity. Repeated local names intentionally overwrite this
                // compatibility view; block-local diagnostics never do.
                let endpoint_state = EndpointValueState {
                    direction: endpoint.direction,
                    dpt: endpoint.dpt,
                    address,
                    observed: None,
                    requested: None,
                };
                block_endpoint_values.insert(
                    (block_id.clone(), endpoint.name.clone()),
                    endpoint_state.clone(),
                );
                endpoint_values.insert(endpoint.name.clone(), endpoint_state);
            }
        }
        let automation = automation_snapshot(runtime);
        let signals = signal_snapshots(runtime);
        let external_inputs = external_inputs_snapshot(runtime);
        let block_health_runtime: BTreeMap<_, _> = blocks
            .keys()
            .cloned()
            .map(|block_id| (block_id, BlockHealthRuntime::default()))
            .collect();
        let mut operations = operations_snapshot(crate::HostLimits::desktop());
        operations.core = core_usage_snapshot(
            logiksmith_core::RuntimeProfile::Desktop,
            logiksmith_core::Runtime::new(runtime.core_config.clone()).usage(),
        );
        operations.block_health = initial_block_health(&blocks);
        Self {
            inner: Arc::new(Mutex::new(Inner {
                revision: 0,
                captured_at_ms: 0,
                connection: ConnectionState::Starting,
                automation_path,
                automation,
                active_document: runtime.document.clone(),
                saved_document: runtime.document.clone(),
                active_automation_revision: revision,
                saved_automation_revision: revision,
                endpoint_values,
                last_write: WriteSnapshot {
                    status: WriteStatus::Idle,
                    request_id: None,
                    block_id: None,
                    execution_id: None,
                    value: None,
                    error: None,
                },
                active_logic_revision: runtime
                    .blocks
                    .first()
                    .map(|block| block.revision)
                    .unwrap_or(1),
                saved_logic_revision: runtime
                    .blocks
                    .first()
                    .map(|block| block.revision)
                    .unwrap_or(1),
                active_structural_revision: runtime.structural_revision,
                saved_structural_revision: runtime.structural_revision,
                restart_required: false,
                state: BTreeMap::new(),
                pending_timers: Vec::new(),
                executions: VecDeque::new(),
                next_execution_id: 1,
                telegrams: VecDeque::new(),
                logs: VecDeque::new(),
                journal: VecDeque::new(),
                pending_writes: BTreeMap::new(),
                blocks,
                block_order: runtime
                    .blocks
                    .iter()
                    .map(|block| block.id.to_string())
                    .collect(),
                block_automation,
                block_endpoint_values,
                site_time: site_time_snapshot(&runtime.core_config.site),
                schedule_status: BTreeMap::new(),
                block_schedules,
                signals,
                external_inputs,
                last_clock_sample: None,
                operations,
                block_health_runtime,
                limits: crate::HostLimits::desktop(),
            })),
            events,
            origin: Instant::now(),
        }
    }
    /// Selects the host profile shown in Operations and establishes its
    /// bounded queue capacities before the session starts admitting work.
    pub fn set_host_limits(&self, limits: crate::HostLimits) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = inner.operations.clone();
        let mut next = operations_snapshot(limits);
        next.fatal = previous.fatal;
        next.status = previous.status;
        next.host_turn = previous.host_turn;
        next.block_health = previous.block_health;
        next.core = core_usage_snapshot(
            limits.profile,
            logiksmith_core::RuntimeUsage {
                logic_blocks: previous.core.logic_blocks.used,
                signals: previous.core.signals.used,
                signal_bindings: previous.core.signal_bindings.used,
                logic_source_bytes: previous.core.logic_source_bytes.used,
                state_entries: previous.core.state_entries.used,
                state_bytes: previous.core.state_bytes.used,
                pending_timers: previous.core.pending_timers.used,
            },
        );
        next.pending_knx_writes = previous.pending_knx_writes;
        next.pending_write_timeouts = previous.pending_write_timeouts;
        inner.operations = next;
        inner.limits = limits;
        while inner.telegrams.len() > limits.recent_telegrams {
            inner.telegrams.pop_front();
        }
        while inner.logs.len() > limits.runtime_logs {
            inner.logs.pop_front();
        }
        while inner.journal.len() > limits.diagnostic_journal {
            inner.journal.pop_front();
        }
        for block in inner.blocks.values_mut() {
            while block.executions.len() > limits.execution_history_per_block {
                block.executions.pop_front();
            }
        }
        self.publish_locked(&mut inner);
    }

    pub fn record_queue_admitted(&self, lane: &str, depth: usize) {
        self.update_operations(|operations| {
            if let Some(queue) = operations.queues.get_mut(lane) {
                queue.depth = depth;
                queue.high_water = queue.high_water.max(depth);
                queue.accepted = queue.accepted.saturating_add(1);
            }
        });
    }

    pub fn record_queue_rejected(&self, lane: &str, depth: usize, fatal: bool) {
        self.update_operations(|operations| {
            if let Some(queue) = operations.queues.get_mut(lane) {
                queue.depth = depth;
                queue.high_water = queue.high_water.max(depth);
                queue.rejected = queue.rejected.saturating_add(1);
            }
            if fatal {
                operations.status = "overloaded".to_owned();
            }
        });
    }

    pub fn set_pending_knx_writes(&self, depth: usize) {
        self.update_operations(|operations| {
            operations.pending_knx_writes = depth;
        });
    }

    pub fn record_pending_write_timeout(&self, request_id: u64) {
        self.update_operations(|operations| {
            operations.pending_write_timeouts = operations.pending_write_timeouts.saturating_add(1);
            operations.status = "degraded".to_owned();
            operations.fatal = Some(format!("pending KNX write timed out request_id={request_id}"));
        });
    }

    pub fn record_runtime_fatal(&self, reason: impl Into<String>, overloaded: bool) {
        self.update_operations(|operations| {
            operations.fatal = Some(reason.into());
            operations.status = if overloaded { "overloaded" } else { "degraded" }.to_owned();
        });
    }

    /// Records one serial host turn against the optional embedded/OpenKNX
    /// timing thresholds.  Desktop's profile deliberately has no threshold;
    /// it still exposes the measured duration and maximum for diagnostics.
    pub fn record_host_turn(&self, duration_us: u64, limits: crate::HostLimits) {
        let runtime_limits = limits.profile.limits();
        let over_budget = runtime_limits
            .signal_cascade_time_budget_ms
            .is_some_and(|budget_ms| duration_us > budget_ms.saturating_mul(1_000));
        let warning = runtime_limits
            .openknx_loop_warning_threshold_ms
            .is_some_and(|threshold_ms| duration_us > threshold_ms.saturating_mul(1_000));
        self.update_operations(|operations| {
            operations.host_turn.last_duration_us = duration_us;
            operations.host_turn.max_duration_us =
                operations.host_turn.max_duration_us.max(duration_us);
            operations.host_turn.last_over_budget = over_budget;
            operations.host_turn.last_warning = warning;
            if over_budget {
                operations.host_turn.over_budget_count =
                    operations.host_turn.over_budget_count.saturating_add(1);
            }
            if warning {
                operations.host_turn.warning_count =
                    operations.host_turn.warning_count.saturating_add(1);
            }
        });
        if warning {
            tracing::warn!(
                target: "logiksmith",
                duration_us,
                threshold_ms = runtime_limits.openknx_loop_warning_threshold_ms.unwrap_or_default(),
                "host turn exceeded OpenKNX loop warning threshold"
            );
        }
    }

    fn update_operations(&self, update: impl FnOnce(&mut OperationsSnapshot)) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        update(&mut inner.operations);
        self.publish_locked(&mut inner);
    }

    pub fn now(&self) -> logiksmith_core::MonotonicMs {
        logiksmith_core::MonotonicMs(
            u64::try_from(self.origin.elapsed().as_millis()).unwrap_or(u64::MAX),
        )
    }
    pub fn snapshot(&self) -> Snapshot {
        self.snapshot_at(self.now())
    }
    pub fn snapshot_at(&self, now: logiksmith_core::MonotonicMs) -> Snapshot {
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        snapshot_locked(&inner, now)
    }
    pub fn latest_revision(&self) -> u64 {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .revision
    }
    pub fn automation_path(&self) -> PathBuf {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .automation_path
            .clone()
    }
    /// Returns the document currently active in the serialized runtime; saved
    /// structural changes can remain pending restart.
    pub fn active_document(&self) -> crate::AutomationDocument {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .active_document
            .clone()
    }
    /// Returns the persisted revision of the active block source used by the
    /// browser-facing simulation contract.
    pub fn active_block_revision(&self, block_id: &str) -> Option<u64> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .active_document
            .blocks
            .iter()
            .find(|block| block.id == block_id)
            .map(|block| block.revision.max(1))
    }
    pub fn set_connection(&self, state: ConnectionState) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if inner.connection == state {
            return;
        }
        inner.connection = state;
        self.publish_locked(&mut inner);
    }
    pub fn set_saved_automation_revision(&self, revision: u64) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if inner.saved_automation_revision == revision {
            return;
        }
        inner.saved_automation_revision = revision;
        self.publish_locked(&mut inner);
    }
    /// Replaces the site wall-clock/astronomy card after a fresh sample.
    pub fn set_site_time(&self, snapshot: SiteTimeSnapshot) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if inner.site_time == snapshot {
            return;
        }
        inner.site_time = snapshot;
        self.publish_locked(&mut inner);
    }
    /// Stores the site projection and its paired wall/monotonic sample.
    pub fn set_site_time_sample(&self, sample: ClockSample, snapshot: SiteTimeSnapshot) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if inner.site_time == snapshot && inner.last_clock_sample == Some(sample) {
            return;
        }
        inner.site_time = snapshot;
        inner.last_clock_sample = Some(sample);
        self.publish_locked(&mut inner);
    }
    /// Refreshes per-schedule scheduler status after a poll or restart.
    pub fn set_schedule_statuses(&self, statuses: Vec<ScheduleStatusFeed>) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut next = BTreeMap::new();
        for status in statuses {
            next.insert((status.block_id.clone(), status.name.clone()), status);
        }
        if inner.schedule_status == next {
            return;
        }
        inner.schedule_status = next;
        self.publish_locked(&mut inner);
    }
    pub fn set_saved_logic_state(
        &self,
        automation_revision: u64,
        logic_revision: u64,
        structural_revision: u64,
        restart_required: bool,
    ) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.saved_automation_revision = automation_revision;
        inner.saved_logic_revision = logic_revision;
        inner.saved_structural_revision = structural_revision;
        inner.restart_required = restart_required;
        self.publish_locked(&mut inner);
    }
    pub fn set_saved_document_state(
        &self,
        automation_revision: u64,
        structural_revision: u64,
        restart_required: bool,
        document: &crate::AutomationDocument,
    ) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.saved_document = document.clone();
        inner.saved_automation_revision = automation_revision;
        inner.saved_structural_revision = structural_revision;
        inner.restart_required = restart_required;
        for candidate in &document.blocks {
            if let Some(block) = inner.blocks.get_mut(&candidate.id) {
                block.saved_enabled = candidate.enabled;
                block.saved_logic_revision = candidate.revision.max(1);
            }
        }
        if !restart_required {
            inner.active_document = document.clone();
            inner.active_automation_revision = automation_revision;
            update_active_document_projection_locked(&mut inner, document);
            inner.active_logic_revision = first_block_revision(document);
            for candidate in &document.blocks {
                if let Some(block) = inner.blocks.get_mut(&candidate.id) {
                    block.active_logic_revision = candidate.revision.max(1);
                    block.active_enabled = candidate.enabled;
                }
            }
        }
        inner.saved_logic_revision = first_block_revision(document);
        self.publish_locked(&mut inner);
    }
    pub fn set_active_logic(&self, logic_revision: u64, source: impl Into<String>) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.active_logic_revision = logic_revision;
        inner.active_automation_revision = logic_revision;
        inner.automation.logic.source = source.into();
        inner.restart_required =
            inner.saved_structural_revision != inner.active_structural_revision;
        self.publish_locked(&mut inner);
    }
    /// Publishes a source activation together with its cancelled timer names
    /// and the new core projection in one SSE update.
    pub fn record_activation(
        &self,
        logic_revision: u64,
        source: impl Into<String>,
        cancelled_timers: &[String],
        snapshot: &logiksmith_core::EngineSnapshot,
    ) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.active_logic_revision = logic_revision;
        inner.active_automation_revision = logic_revision;
        inner.automation.logic.source = source.into();
        inner.restart_required =
            inner.saved_structural_revision != inner.active_structural_revision;
        inner.state = state_record(&snapshot.state);
        inner.pending_timers = snapshot
            .pending_timers
            .iter()
            .map(|timer| pending_timer_record(timer, logic_revision))
            .collect();
        if !cancelled_timers.is_empty() {
            let mut fields = BTreeMap::new();
            fields.insert("cancelled_timers".to_owned(), cancelled_timers.join(","));
            inner.logs.push_back(LogRecord {
                time_ms: self.now().0,
                level: "info".to_owned(),
                target: "logiksmith".to_owned(),
                message: "source activation cancelled pending timers".to_owned(),
                fields,
            });
            while inner.logs.len() > inner.limits.runtime_logs {
                inner.logs.pop_front();
            }
        }
        self.publish_locked(&mut inner);
    }
    /// Replaces the browser projection of core-owned transient state and
    /// pending timers. The session calls this after every serialized runtime
    /// operation, so snapshots and SSE updates share one coherent view.
    pub fn set_runtime_projection(
        &self,
        state: BTreeMap<String, StateValueRecord>,
        pending_timers: Vec<PendingTimerRecord>,
    ) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if inner.state == state && inner.pending_timers == pending_timers {
            return;
        }
        inner.state = state;
        inner.pending_timers = pending_timers;
        self.publish_locked(&mut inner);
    }
    pub fn set_engine_snapshot(&self, snapshot: &logiksmith_core::EngineSnapshot) {
        let logic_revision = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .active_logic_revision;
        self.set_runtime_projection(
            state_record(&snapshot.state),
            snapshot
                .pending_timers
                .iter()
                .map(|timer| pending_timer_record(timer, logic_revision))
                .collect(),
        );
    }
    /// Projects all portable block state into the browser diagnostics view.
    /// Runtime state remains owned by the core; this method only copies its
    /// immutable snapshot into bounded dashboard data.
    pub fn set_runtime_projection_from_runtime(
        &self,
        runtime: &Runtime,
        now: logiksmith_core::MonotonicMs,
    ) {
        let snapshot = runtime.snapshot_at(now);
        let usage = runtime.usage();
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.operations.core = core_usage_snapshot(inner.limits.profile, usage);
        for block in &snapshot.blocks {
            let id = block.id.to_string();
            let revision = inner
                .blocks
                .get(&id)
                .map(|current| current.active_logic_revision)
                .unwrap_or(1);
            let entry = inner
                .blocks
                .entry(id.clone())
                .or_insert_with(|| BlockDiagnosticState {
                    active_enabled: block.enabled,
                    saved_enabled: block.enabled,
                    active_logic_revision: revision,
                    saved_logic_revision: revision,
                    source: String::new(),
                    state: BTreeMap::new(),
                    pending_timers: Vec::new(),
                    executions: VecDeque::new(),
                    last_result: None,
                });
            entry.active_enabled = block.enabled;
            entry.state = state_record(&block.state);
            entry.pending_timers = block
                .pending_timers
                .iter()
                .map(|timer| pending_timer_record(timer, revision))
                .collect();
            for input in &block.inputs {
                if let Some(value) = input.value {
                    if let Some(state) = inner
                        .block_endpoint_values
                        .get_mut(&(id.clone(), input.endpoint.clone()))
                    {
                        state.observed = Some(ValueMessage::from_core(value));
                    }
                }
            }
            let previous_error = inner
                .operations
                .block_health
                .get(&id)
                .and_then(|health| health.last_error.clone());
            let health_runtime = inner
                .block_health_runtime
                .entry(id.clone())
                .or_default();
            health_runtime.consecutive_failures = block.consecutive_failures;
            health_runtime.executions_ms.clear();
            for _ in 0..block.live_executions_in_window {
                health_runtime.executions_ms.push_back(now.0);
            }
            if block.consecutive_failures == 0 {
                health_runtime.last_error = previous_error.clone();
            }
            let last_execution_at_ms = health_runtime.last_execution_at_ms;
            let last_failure_at_ms = health_runtime.last_failure_at_ms;
            let last_error = health_runtime.last_error.clone().or(previous_error);
            let last_suspension = block.last_suspension.map(|suspension| match suspension {
                logiksmith_core::BlockSuspension::ScriptFailures => "script_failures",
                logiksmith_core::BlockSuspension::EventRate => "event_rate",
            }.to_owned());
            inner.operations.block_health.insert(
                id,
                BlockHealthSnapshot {
                    status: block.health.as_str().to_owned(),
                    consecutive_failures: block.consecutive_failures,
                    live_executions_last_second: u32::try_from(block.live_executions_in_window)
                        .unwrap_or(u32::MAX),
                    last_suspension,
                    last_execution_at_ms,
                    last_failure_at_ms,
                    last_error,
                },
            );
        }
        let previous_signals = inner.signals.clone();
        let structural_revision = inner.active_structural_revision;
        inner.signals = snapshot
            .signals
            .iter()
            .map(|signal| {
                signal_snapshot_record(
                    signal,
                    previous_signals.iter().find(|item| item.name == signal.name.as_str()),
                    structural_revision,
                )
            })
            .collect();
        let first_revision = inner
            .block_order
            .first()
            .and_then(|id| inner.blocks.get(id))
            .map(|block| block.active_logic_revision);
        if let Some(first_revision) = first_revision {
            inner.active_logic_revision = first_revision;
            inner.active_automation_revision = first_revision;
        }
        self.publish_locked(&mut inner);
    }
    /// Records one tagged execution in the owning block history.
    pub fn record_block_execution(
        &self,
        execution: &BlockExecution,
        now: logiksmith_core::MonotonicMs,
        duration_us: u64,
        automation: &AutomationRuntime,
        schedule_handling: Option<ScheduleHandling>,
    ) {
        self.record_block_execution_with_origin(
            execution,
            now,
            duration_us,
            automation,
            schedule_handling,
            None,
        );
    }
    pub fn record_block_execution_with_origin(
        &self,
        execution: &BlockExecution,
        now: logiksmith_core::MonotonicMs,
        duration_us: u64,
        automation: &AutomationRuntime,
        schedule_handling: Option<ScheduleHandling>,
        origin: Option<ExecutionOrigin>,
    ) {
        let block_id = execution.block_id.to_string();
        let semantic = &execution.execution;
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // Execution records carry the persisted public block revision. The
        // core's source hash is only used internally for timer ownership and
        // never crosses this diagnostic/API boundary.
        let revision = inner
            .active_document
            .blocks
            .iter()
            .find(|block| block.id == block_id)
            .map(|block| block.revision.max(1))
            .or_else(|| {
                inner
                    .blocks
                    .get(&block_id)
                    .map(|block| block.active_logic_revision)
            })
            .unwrap_or(1);
        let signal_effects = signal_effect_records(&semantic.signal_effects);
        let (status, effects, transition, error) = match &semantic.outcome {
            Ok(effects) => (
                LogicExecutionStatus::Succeeded,
                effects
                    .outputs
                    .iter()
                    .filter_map(|effect| {
                        effect_record_for_block(effect, automation, &execution.block_id)
                    })
                    .collect(),
                Some({
                    let mut transition =
                        transition_record_for_block(effects, automation, &execution.block_id);
                    transition.signal_effects = signal_effects.clone();
                    transition
                }),
                None,
            ),
            Err(error) => (
                LogicExecutionStatus::Failed,
                Vec::new(),
                None,
                Some(logic_error_record(error)),
            ),
        };
        let failed = semantic.outcome.is_err();
        let health_error = error.clone();
        let execution_id = unique_execution_id(&inner, semantic.id);
        inner.next_execution_id = inner.next_execution_id.max(execution_id.saturating_add(1));
        let first_block_id = inner.block_order.first().cloned();
        let causal_producer_execution_id = semantic.causal_producer;
        let causal_producer_block_id = causal_producer_execution_id.and_then(|id| {
            inner
                .blocks
                .iter()
                .find(|(_, block)| block.executions.iter().any(|record| record.execution_id == id))
                .map(|(block_id, _)| block_id.clone())
        });
        let causal_signal = causal_producer_execution_id.and_then(|id| {
            signal_for_causal_execution(&inner, id, &block_id, &semantic.trigger)
        });
        let causal_links = causal_producer_execution_id
            .map(|producer_execution_id| {
                vec![CausalLinkSnapshot {
                    producer_execution_id,
                    consumer_execution_id: execution_id,
                    signal: causal_signal.clone(),
                    producer_block_id: causal_producer_block_id.clone(),
                    consumer_block_id: Some(block_id.clone()),
                }]
            })
            .unwrap_or_default();
        let record = ExecutionRecord {
            block_id: execution.block_id.to_string(),
            execution_id,
            time_ms: now.0,
            duration_us,
            logic_revision: revision,
            status,
            trigger: trigger_record(&semantic.trigger, revision, schedule_handling),
            time_context: time_context_record(&semantic.time_context),
            inputs: semantic.inputs.iter().map(input_snapshot_record).collect(),
            state_before: state_record(&semantic.state_before),
            state_after: state_record(&semantic.state_after),
            timer_effects: transition
                .as_ref()
                .map(|transition| transition.timers.clone())
                .unwrap_or_default(),
            transition,
            effects,
            signal_effects,
            causal_producer_execution_id,
            causal_producer_block_id,
            causal_signal,
            causal_links,
            origin,
            error: error.clone(),
        };
        let max_history = inner.limits.execution_history_per_block;
        let (block_revision, block_state, block_pending, block_executions) = {
            let block =
                inner
                    .blocks
                    .entry(block_id.clone())
                    .or_insert_with(|| BlockDiagnosticState {
                        active_enabled: true,
                        saved_enabled: true,
                        active_logic_revision: revision,
                        saved_logic_revision: revision,
                        source: String::new(),
                        state: BTreeMap::new(),
                        pending_timers: Vec::new(),
                        executions: VecDeque::new(),
                        last_result: None,
                    });
            block.active_logic_revision = revision;
            block.state = record.state_after.clone();
            block.pending_timers = semantic
                .pending_timers
                .iter()
                .map(|timer| pending_timer_record(timer, revision))
                .collect();
            block.last_result = Some(LastResultSnapshot {
                status,
                execution_id,
                time_ms: now.0,
                error,
            });
            block.executions.push_back(record);
            while block.executions.len() > max_history {
                block.executions.pop_front();
            }
            (
                block.active_logic_revision,
                block.state.clone(),
                block.pending_timers.clone(),
                block.executions.clone(),
            )
        };
        if first_block_id.as_deref() == Some(block_id.as_str()) {
            inner.active_logic_revision = block_revision;
            inner.active_automation_revision = block_revision;
            inner.state = block_state;
            inner.pending_timers = block_pending;
            inner.executions = block_executions;
        }
        let health_snapshot = {
            let health_runtime = inner
                .block_health_runtime
                .entry(block_id.clone())
                .or_default();
            while health_runtime
                .executions_ms
                .front()
                .is_some_and(|timestamp| *timestamp < now.0.saturating_sub(1_000))
            {
                health_runtime.executions_ms.pop_front();
            }
            health_runtime.executions_ms.push_back(now.0);
            health_runtime.last_execution_at_ms = Some(now.0);
            if failed {
                health_runtime.consecutive_failures = health_runtime.consecutive_failures.saturating_add(1);
                health_runtime.last_failure_at_ms = Some(now.0);
                health_runtime.last_error = health_error;
            } else {
                health_runtime.consecutive_failures = 0;
                health_runtime.last_error = None;
            }
            BlockHealthSnapshot {
                status: if health_runtime.consecutive_failures >= 5 {
                    "failure_threshold_reached".to_owned()
                } else {
                    "active".to_owned()
                },
                consecutive_failures: health_runtime.consecutive_failures,
                live_executions_last_second: u32::try_from(health_runtime.executions_ms.len())
                    .unwrap_or(u32::MAX),
                last_suspension: None,
                last_execution_at_ms: health_runtime.last_execution_at_ms,
                last_failure_at_ms: health_runtime.last_failure_at_ms,
                last_error: health_runtime.last_error.clone(),
            }
        };
        inner.operations.block_health.insert(
            block_id,
            health_snapshot,
        );
        self.publish_locked(&mut inner);
    }
    /// Applies the core's atomic source/enabled activation result to
    /// diagnostics. No block is updated if core activation failed.
    pub fn record_runtime_activation(
        &self,
        document_revision: u64,
        document: &crate::AutomationDocument,
        activation: &logiksmith_core::ActivationResult,
        runtime: &Runtime,
        automation: &AutomationRuntime,
    ) {
        let snapshot = runtime.snapshot();
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.active_document = document.clone();
        inner.active_automation_revision = document_revision;
        update_active_document_projection_locked(&mut inner, document);
        for result in &activation.blocks {
            let id = result.block_id.to_string();
            let Some(block) = inner.blocks.get_mut(&id) else {
                continue;
            };
            block.active_enabled = result.enabled;
            block.active_logic_revision = document
                .blocks
                .iter()
                .find(|candidate| candidate.id == id)
                .map(|candidate| candidate.revision.max(1))
                .unwrap_or(block.active_logic_revision);
            if let Some(core_block) = runtime.block(&result.block_id) {
                block.source = core_block.logic_program().source().to_owned();
            } else if let Some(document_block) = automation
                .document
                .blocks
                .iter()
                .find(|candidate| candidate.id == id)
            {
                block.source = document_block.source.clone();
            }
            if !result.cancelled_timers.is_empty() {
                let mut fields = BTreeMap::new();
                fields.insert("block_id".to_owned(), id.clone());
                fields.insert(
                    "cancelled_timers".to_owned(),
                    result
                        .cancelled_timers
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(","),
                );
                inner.logs.push_back(LogRecord {
                    time_ms: self.now().0,
                    level: "info".to_owned(),
                    target: "logiksmith".to_owned(),
                    message: "block activation cancelled pending timers".to_owned(),
                    fields,
                });
                while inner.logs.len() > inner.limits.runtime_logs {
                    inner.logs.pop_front();
                }
            }
        }
        inner.active_logic_revision = first_block_revision(&inner.active_document);
        drop(inner);
        self.set_runtime_projection_from_runtime(
            runtime,
            runtime.last_accepted_at().unwrap_or_default(),
        );
        let _ = snapshot;
    }
}

include!("diagnostics_store_history.rs");

impl DiagnosticStore {
    fn publish_locked(&self, inner: &mut Inner) {
        inner.revision = inner.revision.saturating_add(1);
        inner.captured_at_ms = self.now().0;
        let update = DiagnosticUpdate {
            revision: inner.revision,
            snapshot: snapshot_locked(inner, self.now()),
        };
        inner.journal.push_back(update.clone());
        while inner.journal.len() > inner.limits.diagnostic_journal {
            inner.journal.pop_front();
        }
        let _ = self.events.send(update);
    }
}

include!("diagnostics_store_transport.rs");
