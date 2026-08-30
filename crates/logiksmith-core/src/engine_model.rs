impl Engine {
    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    pub fn limits(&self) -> RuntimeLimits {
        self.limits
    }

    pub fn logic_program(&self) -> &LogicProgram {
        &self.config.logic
    }

    /// The active source revision. Replacing a source takes effect only after
    /// validation and between calls, so this always identifies the next
    /// execution's program.
    pub fn active_logic_revision(&self) -> LogicRevision {
        self.config.logic.revision
    }

    /// Alias for hosts that use the shorter revision terminology.
    pub fn logic_revision(&self) -> LogicRevision {
        self.active_logic_revision()
    }

    pub fn snapshot(&self) -> EngineSnapshot {
        EngineSnapshot {
            logic_revision: self.active_logic_revision(),
            known_inputs: self.known_input_values(),
            state: self.state.clone(),
            pending_timers: self.pending_timers(),
        }
    }

    pub fn state(&self) -> &TransientState {
        &self.state
    }

    pub fn transient_state(&self) -> &TransientState {
        self.state()
    }

    pub fn pending_timers(&self) -> Vec<PendingTimer> {
        self.pending_timers.values().cloned().collect()
    }

    pub fn next_timer_deadline(&self) -> Option<MonotonicMs> {
        self.pending_timers.values().map(|timer| timer.due_at).min()
    }

    /// Returns known values in configured input declaration order.
    pub fn known_input_values(&self) -> Vec<(EndpointName, TypedValue)> {
        self.config
            .endpoints
            .iter()
            .enumerate()
            .filter_map(|(index, endpoint)| {
                (endpoint.direction == EndpointDirection::Input)
                    .then(|| {
                        self.inputs[index]
                            .value
                            .map(|value| (endpoint.name.clone(), value))
                    })
                    .flatten()
            })
            .collect()
    }
}
