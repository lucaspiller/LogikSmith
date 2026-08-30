impl Engine {
    /// Applies one transport-neutral input update without requiring a
    /// [`ClockSample`]. This is the compatibility form for hosts that only
    /// have a monotonic clock.
    pub fn process_input_update(
        &mut self,
        endpoint: EndpointName,
        update: InputUpdate,
        now: MonotonicMs,
    ) -> Result<Option<Execution>, EventError> {
        self.process_input_update_with_context(endpoint, update, now, &default_site(), None, None)
    }
    /// Applies one transport-neutral input update with an explicit host clock
    /// sample. Observe and invalidate updates do not evaluate Lua.
    pub fn process_input_update_sampled(
        &mut self,
        endpoint: EndpointName,
        update: InputUpdate,
        sample: ClockSample,
        site: &SiteTimeConfig,
    ) -> Result<Option<Execution>, EventError> {
        self.process_input_update_with_context(
            endpoint,
            update,
            sample.monotonic_ms,
            site,
            sample.utc_unix_ms,
            None,
        )
    }

    /// ClockSample variant of [`Self::process_input_with_budget_probe`]. The
    /// host supplies the same elapsed-time source it uses for the enclosing
    /// handler, allowing embedded profiles to stop before commit.
    pub fn process_input_update_sampled_with_budget_probe(
        &mut self,
        endpoint: EndpointName,
        update: InputUpdate,
        sample: ClockSample,
        site: &SiteTimeConfig,
        budget_probe: BudgetProbeHandle,
    ) -> Result<Option<Execution>, EventError> {
        self.process_input_update_with_context(
            endpoint,
            update,
            sample.monotonic_ms,
            site,
            sample.utc_unix_ms,
            Some(budget_probe),
        )
    }

    /// Applies a triggering input while consulting a host-owned elapsed-time
    /// probe. This is primarily used by the embedded host; normal desktop
    /// callers can keep using [`Self::process_input`].
    pub fn process_input_with_budget_probe(
        &mut self,
        event: InputEvent,
        now: MonotonicMs,
        budget_probe: BudgetProbeHandle,
    ) -> Result<Execution, EventError> {
        self.process_input_update_with_context(
            event.endpoint,
            InputUpdate::Trigger(event.value),
            now,
            &default_site(),
            None,
            Some(budget_probe),
        )?
        .ok_or_else(|| unreachable!("trigger input always produces an execution"))
    }

    fn process_input_update_with_context(
        &mut self,
        endpoint: EndpointName,
        update: InputUpdate,
        now: MonotonicMs,
        site: &SiteTimeConfig,
        utc_unix_ms: Option<i64>,
        budget_probe: Option<BudgetProbeHandle>,
    ) -> Result<Option<Execution>, EventError> {
        let index = match update {
            InputUpdate::Observe(value) | InputUpdate::Trigger(value) => {
                self.validate_input(&endpoint, value)?
            }
            InputUpdate::Invalidate => self.validate_endpoint(&endpoint)?,
        };
        self.accept_time(now)?;
        match update {
            InputUpdate::Observe(value) => {
                self.inputs[index] = InputState { value: Some(value), observed_at: Some(now) };
                Ok(None)
            }
            InputUpdate::Invalidate => {
                self.inputs[index] = InputState::default();
                Ok(None)
            }
            InputUpdate::Trigger(value) => {
                let previous = self.inputs[index].value;
                self.inputs[index] = InputState { value: Some(value), observed_at: Some(now) };
                let trigger = input_trigger(endpoint, value, previous);
                let snapshots = self.input_snapshots(now);
                let state_before = self.state.clone();
                let outcome = execute_logic(
                    &self.config.endpoints,
                    &self.config.logic,
                    &snapshots,
                    &Trigger::Input(trigger.clone()),
                    &state_before,
                    &self.pending_timers,
                    now,
                    &TimeContext::capture(site, utc_unix_ms),
                    &self.limits,
                    budget_probe,
                );
                let mut state_after = state_before.clone();
                let mut pending_timers = self.pending_timers();
                if let Ok(transition) = &outcome {
                    state_after = merge_state_with_limits(&state_before, &transition.state, &self.limits)
                        .expect("validated transition state must merge");
                    let candidate = apply_timer_effects(
                        &self.pending_timers,
                        &transition.timers,
                        now,
                        self.active_logic_revision(),
                    );
                    pending_timers = candidate.values().cloned().collect();
                    self.state = state_after.clone();
                    self.pending_timers = candidate;
                }
                Ok(Some(Execution::with_now(
                    self.active_logic_revision(),
                    Trigger::Input(trigger),
                    snapshots,
                    state_before,
                    state_after,
                    pending_timers,
                    outcome,
                    site,
                    utc_unix_ms,
                )))
            }
        }
    }
}
