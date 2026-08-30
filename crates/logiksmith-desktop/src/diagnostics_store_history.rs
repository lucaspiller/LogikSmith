impl DiagnosticStore {
    /// Stores one immutable semantic core execution with host-only timing and
    /// resolved KNX destinations. The core outcome is intentionally handled
    /// here so zero-effect successes and contained Lua failures are retained.
    pub fn record_execution(
        &self,
        execution: &Execution,
        duration_us: u64,
        automation: &AutomationRuntime,
    ) {
        self.record_execution_at(execution, self.now(), duration_us, automation);
    }
    pub fn record_execution_at(
        &self,
        execution: &Execution,
        now: logiksmith_core::MonotonicMs,
        duration_us: u64,
        automation: &AutomationRuntime,
    ) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let execution_id = unique_execution_id(&inner, execution.id);
        inner.next_execution_id = inner.next_execution_id.max(execution_id.saturating_add(1));
        let document_revision = inner.active_logic_revision;
        let signal_effects = signal_effect_records(&execution.signal_effects);
        let (status, effects, transition, error) = match &execution.outcome {
            Ok(effects) => (
                LogicExecutionStatus::Succeeded,
                effects
                    .outputs
                    .iter()
                    .filter_map(|effect| effect_record(effect, automation))
                    .collect(),
                Some({
                    let mut transition = transition_record(effects, automation);
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
        let block_id = automation
            .blocks
            .first()
            .map(|block| block.id.to_string())
            .unwrap_or_default();
        let causal_producer_execution_id = execution.causal_producer;
        let causal_producer_block_id = causal_producer_execution_id.and_then(|id| {
            inner
                .executions
                .iter()
                .find(|record| record.execution_id == id)
                .map(|record| record.block_id.clone())
        });
        let causal_links = causal_producer_execution_id
            .map(|producer_execution_id| {
                vec![CausalLinkSnapshot {
                    producer_execution_id,
                    consumer_execution_id: execution_id,
                    signal: None,
                    producer_block_id: causal_producer_block_id.clone(),
                    consumer_block_id: Some(block_id.clone()),
                }]
            })
            .unwrap_or_default();
        inner.executions.push_back(ExecutionRecord {
            block_id,
            execution_id,
            time_ms: now.0,
            duration_us,
            logic_revision: document_revision,
            status,
            trigger: trigger_record(&execution.trigger, document_revision, None),
            time_context: time_context_record(&execution.time_context),
            inputs: execution.inputs.iter().map(input_snapshot_record).collect(),
            state_before: state_record(&execution.state_before),
            state_after: state_record(&execution.state_after),
            timer_effects: transition
                .as_ref()
                .map(|transition| transition.timers.clone())
                .unwrap_or_default(),
            transition,
            effects,
            signal_effects,
            causal_producer_execution_id,
            causal_producer_block_id,
            causal_signal: None,
            causal_links,
            origin: None,
            error,
        });
        inner.state = state_record(&execution.state_after);
        inner.pending_timers = execution
            .pending_timers
            .iter()
            .map(|timer| pending_timer_record(timer, document_revision))
            .collect();
        while inner.executions.len() > inner.limits.execution_history_per_block {
            inner.executions.pop_front();
        }
        self.publish_locked(&mut inner);
    }
    pub fn record_log(
        &self,
        level: impl Into<String>,
        target: impl Into<String>,
        message: impl Into<String>,
        fields: BTreeMap<String, String>,
    ) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.logs.push_back(LogRecord {
            time_ms: self.now().0,
            level: level.into(),
            target: target.into(),
            message: message.into(),
            fields,
        });
        while inner.logs.len() > inner.limits.runtime_logs {
            inner.logs.pop_front();
        }
        self.publish_locked(&mut inner);
    }
    pub fn subscribe(&self, since: Option<u64>) -> EventSubscription {
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let receiver = self.events.subscribe();
        let since = since.unwrap_or(0);
        let replay = match inner.journal.front().map(|update| update.revision) {
            Some(first) if since.saturating_add(1) < first => Replay::Resync {
                revision: inner.revision,
            },
            _ => Replay::Updates(
                inner
                    .journal
                    .iter()
                    .filter(|update| update.revision > since)
                    .cloned()
                    .collect(),
            ),
        };
        EventSubscription { replay, receiver }
    }
}
