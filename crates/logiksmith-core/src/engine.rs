use std::collections::BTreeMap;

use crate::lua::execute_logic;
use crate::state::{
    apply_timer_effects, merge_state, validate_pending_timer_map, validate_pending_timers,
    validate_state_map,
};
use crate::support::{
    default_site, input_trigger, unavailable_time_context, validate_simulation_input,
    validate_simulation_value,
};
use crate::*;

/// The core event-to-Lua-to-effect engine.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Engine {
    pub(crate) config: EngineConfig,
    inputs: Vec<InputState>,
    state: TransientState,
    pub(crate) pending_timers: BTreeMap<TimerName, PendingTimer>,
    last_accepted_at: Option<MonotonicMs>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct InputState {
    value: Option<TypedValue>,
    observed_at: Option<MonotonicMs>,
}

impl Engine {
    /// Constructs an engine, panicking if the configuration is invalid.
    /// Prefer [`Self::try_new`] at an external configuration boundary.
    pub fn new(config: EngineConfig) -> Self {
        Self::try_new(config).expect("invalid LogikSmith core configuration")
    }

    pub fn try_new(config: EngineConfig) -> Result<Self, ConfigError> {
        config.validate()?;
        let inputs = vec![InputState::default(); config.endpoints.len()];
        Ok(Self {
            config,
            inputs,
            state: BTreeMap::new(),
            pending_timers: BTreeMap::new(),
            last_accepted_at: None,
        })
    }

    pub fn config(&self) -> &EngineConfig {
        &self.config
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

    /// Validates a candidate source in the same restricted environment used at
    /// execution time, without changing the active program.
    pub fn validate_source(source: &str) -> Result<LogicRevision, LogicError> {
        let program = LogicProgram::try_new(source.to_owned())?;
        Ok(program.revision)
    }

    /// Validates and atomically activates a source for the next execution.
    pub fn replace_source(
        &mut self,
        source: impl Into<String>,
    ) -> Result<LogicRevision, LogicError> {
        Ok(self.activate_source(source)?.logic_revision)
    }

    /// Validates and activates source, preserving state and cancelling timers
    /// from a previous revision as one atomic operation.
    pub fn activate_source(
        &mut self,
        source: impl Into<String>,
    ) -> Result<SourceActivation, LogicError> {
        let program = LogicProgram::try_new(source)?;
        let revision = program.revision;
        if revision == self.active_logic_revision() {
            return Ok(SourceActivation {
                logic_revision: revision,
                cancelled_timers: Vec::new(),
                changed: false,
            });
        }
        let cancelled_timers = self.pending_timers.keys().cloned().collect();
        self.pending_timers.clear();
        self.config.logic = program;
        Ok(SourceActivation {
            logic_revision: revision,
            cancelled_timers,
            changed: true,
        })
    }

    pub fn activate_logic_source(
        &mut self,
        source: impl Into<String>,
    ) -> Result<SourceActivation, LogicError> {
        self.activate_source(source)
    }

    pub fn activate(&mut self, source: impl Into<String>) -> Result<SourceActivation, LogicError> {
        self.activate_source(source)
    }

    pub fn replace_source_with_cancellations(
        &mut self,
        source: impl Into<String>,
    ) -> Result<SourceActivation, LogicError> {
        self.activate_source(source)
    }

    /// Alias emphasizing that this replaces the active logic block.
    pub fn replace_logic_source(
        &mut self,
        source: impl Into<String>,
    ) -> Result<LogicRevision, LogicError> {
        self.replace_source(source)
    }

    /// Compatibility entry point for the desktop activation channel. The
    /// source itself remains the revision authority; the supplied revision is
    /// accepted as the host's stale-write token after source validation.
    pub fn replace_logic(
        &mut self,
        source: impl Into<String>,
        revision: LogicRevision,
    ) -> Result<(), LogicError> {
        let program = LogicProgram::try_new(source)?;
        if program.revision != revision {
            return Err(LogicError::Load {
                message: "logic source revision does not match its source bytes".to_owned(),
                line: None,
            });
        }
        if program.revision != self.active_logic_revision() {
            self.pending_timers.clear();
            self.config.logic = program;
        }
        Ok(())
    }

    pub fn replace_logic_with_cancellations(
        &mut self,
        source: impl Into<String>,
        revision: LogicRevision,
    ) -> Result<SourceActivation, LogicError> {
        let source = source.into();
        let program = LogicProgram::try_new(source.clone())?;
        if program.revision != revision {
            return Err(LogicError::Load {
                message: "logic source revision does not match its source bytes".to_owned(),
                line: None,
            });
        }
        self.activate_source(source)
    }

    /// Records a value-carrying observation without invoking Lua.
    pub fn observe_input(
        &mut self,
        observation: InputObservation,
        now: MonotonicMs,
    ) -> Result<(), EventError> {
        let index = self.validate_input(&observation.endpoint, observation.value)?;
        self.accept_time(now)?;
        self.inputs[index] = InputState {
            value: Some(observation.value),
            observed_at: Some(now),
        };
        Ok(())
    }

    /// Updates the triggering input before evaluating the active source.
    /// The legacy MonotonicMs variant captures an unavailable time context
    /// (no wall-clock instant is supplied).
    pub fn process_input(
        &mut self,
        event: InputEvent,
        now: MonotonicMs,
    ) -> Result<Execution, EventError> {
        self.process_input_with_context(event, now, &default_site(), None)
    }

    /// ClockSample variant: captures the frozen time context from `site` and
    /// the sample's wall-clock instant.
    pub fn process_input_sampled(
        &mut self,
        event: InputEvent,
        sample: ClockSample,
        site: &SiteTimeConfig,
    ) -> Result<Execution, EventError> {
        self.process_input_with_context(event, sample.monotonic_ms, site, sample.utc_unix_ms)
    }

    fn process_input_with_context(
        &mut self,
        event: InputEvent,
        now: MonotonicMs,
        site: &SiteTimeConfig,
        utc_unix_ms: Option<i64>,
    ) -> Result<Execution, EventError> {
        let index = self.validate_input(&event.endpoint, event.value)?;
        self.accept_time(now)?;
        let previous = self.inputs[index].value;
        self.inputs[index] = InputState {
            value: Some(event.value),
            observed_at: Some(now),
        };
        let trigger = input_trigger(event.endpoint, event.value, previous);
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
        );
        let mut state_after = state_before.clone();
        let mut pending_timers = self.pending_timers();
        if let Ok(transition) = &outcome {
            state_after = merge_state(&state_before, &transition.state)
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
        Ok(Execution::with_now(
            self.active_logic_revision(),
            Trigger::Input(trigger),
            snapshots,
            state_before,
            state_after,
            pending_timers,
            outcome,
            site,
            utc_unix_ms,
        ))
    }

    /// The legacy MonotonicMs variant captures an unavailable time context.
    pub fn process_next_due_timer(
        &mut self,
        now: MonotonicMs,
    ) -> Result<Option<Execution>, EventError> {
        self.process_next_due_timer_with_context(now, &default_site(), None)
    }

    /// ClockSample variant: captures the frozen time context from `site` and
    /// the sample's wall-clock instant.
    pub fn process_next_due_timer_sampled(
        &mut self,
        sample: ClockSample,
        site: &SiteTimeConfig,
    ) -> Result<Option<Execution>, EventError> {
        self.process_next_due_timer_with_context(sample.monotonic_ms, site, sample.utc_unix_ms)
    }

    fn process_next_due_timer_with_context(
        &mut self,
        now: MonotonicMs,
        site: &SiteTimeConfig,
        utc_unix_ms: Option<i64>,
    ) -> Result<Option<Execution>, EventError> {
        self.accept_time(now)?;
        let Some((name, timer)) = self
            .pending_timers
            .values()
            .filter(|timer| timer.due_at <= now)
            .min_by(|left, right| {
                left.due_at
                    .cmp(&right.due_at)
                    .then_with(|| left.name.cmp(&right.name))
            })
            .map(|timer| (timer.name.clone(), timer.clone()))
        else {
            return Ok(None);
        };
        self.pending_timers.remove(&name);
        if timer.scheduled_logic_revision != self.active_logic_revision() {
            return Err(EventError::StaleTimer {
                timer: name,
                scheduled_logic_revision: timer.scheduled_logic_revision,
                active_logic_revision: self.active_logic_revision(),
            });
        }
        let trigger = TimerTrigger {
            name,
            scheduled_at: timer.scheduled_at,
            due_at: timer.due_at,
            fired_at: now,
            scheduled_logic_revision: timer.scheduled_logic_revision,
        };
        let public_trigger = Trigger::Timer(trigger.clone());
        let snapshots = self.input_snapshots(now);
        let state_before = self.state.clone();
        let outcome = execute_logic(
            &self.config.endpoints,
            &self.config.logic,
            &snapshots,
            &public_trigger,
            &state_before,
            &self.pending_timers,
            now,
            &TimeContext::capture(site, utc_unix_ms),
        );
        let mut state_after = state_before.clone();
        let mut pending_timers = self.pending_timers();
        if let Ok(transition) = &outcome {
            state_after = merge_state(&state_before, &transition.state)
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
            public_trigger,
            snapshots,
            state_before,
            state_after,
            pending_timers,
            outcome,
            site,
            utc_unix_ms,
        )))
    }

    pub fn process_due_timer(&mut self, now: MonotonicMs) -> Result<Option<Execution>, EventError> {
        self.process_next_due_timer(now)
    }

    /// Evaluates the active source against a complete browser-supplied input
    /// scenario without changing any live input state or timestamps.
    pub fn simulate_input(
        &self,
        scenario: SimulationScenario,
    ) -> Result<Execution, SimulationError> {
        // Store references by configured endpoint index. This both rejects
        // duplicates and makes the returned snapshot follow declaration order.
        let mut supplied: Vec<Option<&SimulationInput>> = vec![None; self.config.endpoints.len()];
        for input in &scenario.inputs {
            let Some(index) = self
                .config
                .endpoints
                .iter()
                .position(|endpoint| endpoint.name == input.endpoint)
            else {
                return Err(SimulationError::UnknownEndpoint(input.endpoint.clone()));
            };
            let endpoint = &self.config.endpoints[index];
            if endpoint.direction != EndpointDirection::Input {
                return Err(SimulationError::EndpointNotInput {
                    endpoint: input.endpoint.clone(),
                    actual: endpoint.direction,
                });
            }
            if supplied[index].is_some() {
                return Err(SimulationError::DuplicateInput(input.endpoint.clone()));
            }
            validate_simulation_input(endpoint, input)?;
            supplied[index] = Some(input);
        }

        for (index, endpoint) in self.config.endpoints.iter().enumerate() {
            if endpoint.direction == EndpointDirection::Input && supplied[index].is_none() {
                return Err(SimulationError::MissingInput(endpoint.name.clone()));
            }
        }

        let trigger_index = self
            .config
            .endpoints
            .iter()
            .position(|endpoint| endpoint.name == scenario.trigger.endpoint)
            .ok_or_else(|| SimulationError::UnknownEndpoint(scenario.trigger.endpoint.clone()))?;
        let trigger_endpoint = &self.config.endpoints[trigger_index];
        if trigger_endpoint.direction != EndpointDirection::Input {
            return Err(SimulationError::EndpointNotInput {
                endpoint: scenario.trigger.endpoint.clone(),
                actual: trigger_endpoint.direction,
            });
        }
        validate_simulation_value(trigger_endpoint, scenario.trigger.value)?;
        if let Some(previous) = scenario.trigger.previous {
            validate_simulation_value(trigger_endpoint, previous)?;
        }

        let trigger_input =
            supplied[trigger_index].expect("configured input presence was validated above");
        if !trigger_input.valid {
            return Err(SimulationError::MissingValue(
                scenario.trigger.endpoint.clone(),
            ));
        }
        let actual_trigger_value = trigger_input
            .value
            .expect("valid trigger input value was validated above");
        if actual_trigger_value != scenario.trigger.value {
            return Err(SimulationError::TriggerValueMismatch {
                endpoint: scenario.trigger.endpoint.clone(),
                expected: scenario.trigger.value,
                actual: actual_trigger_value,
            });
        }
        if trigger_input.age_ms != Some(0) {
            return Err(SimulationError::TriggerAgeMismatch {
                endpoint: scenario.trigger.endpoint.clone(),
                actual: trigger_input.age_ms,
            });
        }

        let snapshots = self
            .config
            .endpoints
            .iter()
            .enumerate()
            .filter_map(|(index, endpoint)| {
                (endpoint.direction == EndpointDirection::Input).then(|| {
                    let input =
                        supplied[index].expect("configured input presence was validated above");
                    InputSnapshot {
                        endpoint: endpoint.name.clone(),
                        dpt: endpoint.dpt,
                        value: input.value,
                        valid: input.valid,
                        age_ms: input.age_ms,
                    }
                })
            })
            .collect::<Vec<_>>();
        let trigger = input_trigger(
            scenario.trigger.endpoint,
            scenario.trigger.value,
            scenario.trigger.previous,
        );
        let state_before = self.state.clone();
        let outcome = execute_logic(
            &self.config.endpoints,
            &self.config.logic,
            &snapshots,
            &Trigger::Input(trigger.clone()),
            &state_before,
            &self.pending_timers,
            MonotonicMs(0),
            &unavailable_time_context(),
        );
        let state_after = outcome
            .as_ref()
            .ok()
            .and_then(|transition| merge_state(&state_before, &transition.state).ok())
            .unwrap_or_else(|| state_before.clone());
        let pending_timers = outcome
            .as_ref()
            .ok()
            .map(|transition| {
                apply_timer_effects(
                    &self.pending_timers,
                    &transition.timers,
                    MonotonicMs(0),
                    self.active_logic_revision(),
                )
                .values()
                .cloned()
                .collect()
            })
            .unwrap_or_else(|| self.pending_timers());
        Ok(Execution::with_now(
            self.active_logic_revision(),
            Trigger::Input(trigger),
            snapshots,
            state_before,
            state_after,
            pending_timers,
            outcome,
            &default_site(),
            None,
        ))
    }

    /// Simulates an input using explicit copied state, timers, and execution
    /// time. This is the extension point used by the desktop simulation form.
    pub fn simulate_input_with_state(
        &self,
        scenario: SimulationScenario,
        state: TransientState,
        pending_timers: Vec<PendingTimer>,
        now: MonotonicMs,
    ) -> Result<Execution, SimulationError> {
        validate_state_map(&state).map_err(SimulationError::InvalidState)?;
        validate_pending_timers(&pending_timers, self.active_logic_revision())?;
        let execution = self.simulate_input_against(scenario, state, pending_timers, now)?;
        Ok(execution)
    }

    pub fn simulate_timer(
        &self,
        scenario: TimerSimulationScenario,
    ) -> Result<Execution, SimulationError> {
        validate_state_map(&scenario.state).map_err(SimulationError::InvalidState)?;
        let mut supplied = BTreeMap::new();
        for timer in scenario.pending_timers {
            if supplied.insert(timer.name.clone(), timer.clone()).is_some() {
                return Err(SimulationError::DuplicateTimer(timer.name));
            }
        }
        validate_pending_timer_map(&supplied, self.active_logic_revision())?;
        let timer = supplied
            .remove(&scenario.timer)
            .ok_or_else(|| SimulationError::UnknownTimer(scenario.timer.clone()))?;
        if timer.scheduled_logic_revision != self.active_logic_revision() {
            return Err(SimulationError::TimerRevisionMismatch {
                timer: scenario.timer,
                scheduled: timer.scheduled_logic_revision,
                active: self.active_logic_revision(),
            });
        }
        let snapshots = self.validate_and_build_snapshots(&scenario.inputs)?;
        let trigger = Trigger::Timer(TimerTrigger {
            name: timer.name,
            scheduled_at: timer.scheduled_at,
            due_at: timer.due_at,
            fired_at: scenario.fired_at,
            scheduled_logic_revision: timer.scheduled_logic_revision,
        });
        let state_before = scenario.state;
        let outcome = execute_logic(
            &self.config.endpoints,
            &self.config.logic,
            &snapshots,
            &trigger,
            &state_before,
            &supplied,
            scenario.fired_at,
            &unavailable_time_context(),
        );
        let state_after = outcome
            .as_ref()
            .ok()
            .and_then(|transition| merge_state(&state_before, &transition.state).ok())
            .unwrap_or_else(|| state_before.clone());
        let pending_timers = outcome
            .as_ref()
            .ok()
            .map(|transition| {
                apply_timer_effects(
                    &supplied,
                    &transition.timers,
                    scenario.fired_at,
                    self.active_logic_revision(),
                )
                .values()
                .cloned()
                .collect()
            })
            .unwrap_or_else(|| supplied.values().cloned().collect());
        Ok(Execution::with_now(
            self.active_logic_revision(),
            trigger,
            snapshots,
            state_before,
            state_after,
            pending_timers,
            outcome,
            &default_site(),
            None,
        ))
    }

    /// Evaluates the active source against a delivered schedule trigger. The
    /// trigger was already validated by the runtime (known block, enabled,
    /// current structural revision), so this path cannot produce input
    /// validation errors; Lua failures are contained in the execution.
    pub fn process_schedule_trigger(
        &mut self,
        trigger: ScheduleTrigger,
        site: &SiteTimeConfig,
        utc_unix_ms: Option<i64>,
        now: MonotonicMs,
    ) -> Result<Execution, EventError> {
        let public_trigger = Trigger::Schedule(trigger);
        let snapshots = self.input_snapshots(now);
        let state_before = self.state.clone();
        let outcome = execute_logic(
            &self.config.endpoints,
            &self.config.logic,
            &snapshots,
            &public_trigger,
            &state_before,
            &self.pending_timers,
            now,
            &TimeContext::capture(site, utc_unix_ms),
        );
        let mut state_after = state_before.clone();
        let mut pending_timers = self.pending_timers();
        if let Ok(transition) = &outcome {
            state_after = merge_state(&state_before, &transition.state)
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
        Ok(Execution::with_now(
            self.active_logic_revision(),
            public_trigger,
            snapshots,
            state_before,
            state_after,
            pending_timers,
            outcome,
            site,
            utc_unix_ms,
        ))
    }

    /// Simulates one delivered schedule trigger against the current input
    /// state without mutating anything. The time context is frozen at
    /// `utc_unix_ms`.
    pub fn simulate_schedule_trigger(
        &self,
        trigger: ScheduleTrigger,
        site: &SiteTimeConfig,
        utc_unix_ms: Option<i64>,
    ) -> Execution {
        let snapshots = self.input_snapshots(MonotonicMs(0));
        let state_before = self.state.clone();
        let outcome = execute_logic(
            &self.config.endpoints,
            &self.config.logic,
            &snapshots,
            &Trigger::Schedule(trigger.clone()),
            &state_before,
            &self.pending_timers,
            MonotonicMs(0),
            &TimeContext::capture(site, utc_unix_ms),
        );
        let state_after = outcome
            .as_ref()
            .ok()
            .and_then(|transition| merge_state(&state_before, &transition.state).ok())
            .unwrap_or_else(|| state_before.clone());
        let pending_timers = outcome
            .as_ref()
            .ok()
            .map(|transition| {
                apply_timer_effects(
                    &self.pending_timers,
                    &transition.timers,
                    MonotonicMs(0),
                    self.active_logic_revision(),
                )
                .values()
                .cloned()
                .collect()
            })
            .unwrap_or_else(|| self.pending_timers());
        Execution::with_now(
            self.active_logic_revision(),
            Trigger::Schedule(trigger),
            snapshots,
            state_before,
            state_after,
            pending_timers,
            outcome,
            site,
            utc_unix_ms,
        )
    }

    pub(crate) fn validate_input(
        &self,
        endpoint_name: &EndpointName,
        value: TypedValue,
    ) -> Result<usize, EventError> {
        value.validate().map_err(EventError::InvalidValue)?;
        let (index, endpoint) = self
            .config
            .endpoints
            .iter()
            .enumerate()
            .find(|(_, endpoint)| endpoint.name == *endpoint_name)
            .ok_or_else(|| EventError::UnknownEndpoint(endpoint_name.clone()))?;
        if endpoint.direction != EndpointDirection::Input {
            return Err(EventError::EndpointNotInput {
                endpoint: endpoint_name.clone(),
                actual: endpoint.direction,
            });
        }
        if endpoint.dpt != value.dpt {
            return Err(EventError::DptMismatch {
                endpoint: endpoint_name.clone(),
                expected: endpoint.dpt,
                actual: value.dpt,
            });
        }
        Ok(index)
    }

    fn accept_time(&mut self, now: MonotonicMs) -> Result<(), EventError> {
        if let Some(previous) = self.last_accepted_at
            && now < previous
        {
            return Err(EventError::TimeWentBackwards {
                previous,
                current: now,
            });
        }
        self.last_accepted_at = Some(now);
        Ok(())
    }

    pub(crate) fn input_snapshots(&self, now: MonotonicMs) -> Vec<InputSnapshot> {
        self.config
            .endpoints
            .iter()
            .enumerate()
            .filter_map(|(index, endpoint)| {
                (endpoint.direction == EndpointDirection::Input).then(|| {
                    let state = &self.inputs[index];
                    let age_ms = state
                        .observed_at
                        .map(|observed_at| now.0.saturating_sub(observed_at.0));
                    InputSnapshot {
                        endpoint: endpoint.name.clone(),
                        dpt: endpoint.dpt,
                        value: state.value,
                        valid: state.value.is_some() && state.observed_at.is_some(),
                        age_ms,
                    }
                })
            })
            .collect()
    }

    fn simulate_input_against(
        &self,
        scenario: SimulationScenario,
        state: TransientState,
        pending_timers: Vec<PendingTimer>,
        now: MonotonicMs,
    ) -> Result<Execution, SimulationError> {
        let snapshots = self.validate_and_build_snapshots(&scenario.inputs)?;
        let trigger_index = self
            .config
            .endpoints
            .iter()
            .position(|endpoint| endpoint.name == scenario.trigger.endpoint)
            .ok_or_else(|| SimulationError::UnknownEndpoint(scenario.trigger.endpoint.clone()))?;
        let endpoint = &self.config.endpoints[trigger_index];
        if endpoint.direction != EndpointDirection::Input {
            return Err(SimulationError::EndpointNotInput {
                endpoint: scenario.trigger.endpoint.clone(),
                actual: endpoint.direction,
            });
        }
        validate_simulation_value(endpoint, scenario.trigger.value)?;
        if let Some(previous) = scenario.trigger.previous {
            validate_simulation_value(endpoint, previous)?;
        }
        let supplied = snapshots
            .iter()
            .find(|input| input.endpoint == scenario.trigger.endpoint)
            .ok_or_else(|| SimulationError::MissingInput(scenario.trigger.endpoint.clone()))?;
        if !supplied.valid {
            return Err(SimulationError::MissingValue(
                scenario.trigger.endpoint.clone(),
            ));
        }
        if supplied.value != Some(scenario.trigger.value) {
            return Err(SimulationError::TriggerValueMismatch {
                endpoint: scenario.trigger.endpoint.clone(),
                expected: scenario.trigger.value,
                actual: supplied.value.unwrap_or(scenario.trigger.value),
            });
        }
        if supplied.age_ms != Some(0) {
            return Err(SimulationError::TriggerAgeMismatch {
                endpoint: scenario.trigger.endpoint.clone(),
                actual: supplied.age_ms,
            });
        }
        let trigger = input_trigger(
            scenario.trigger.endpoint,
            scenario.trigger.value,
            scenario.trigger.previous,
        );
        let trigger_kind = Trigger::Input(trigger.clone());
        let state_before = state;
        let mut timer_map = BTreeMap::new();
        for timer in pending_timers {
            let timer_name = timer.name.clone();
            if timer_map.insert(timer_name.clone(), timer).is_some() {
                return Err(SimulationError::DuplicateTimer(timer_name));
            }
        }
        let outcome = execute_logic(
            &self.config.endpoints,
            &self.config.logic,
            &snapshots,
            &trigger_kind,
            &state_before,
            &timer_map,
            now,
            &unavailable_time_context(),
        );
        let state_after = outcome
            .as_ref()
            .ok()
            .and_then(|transition| merge_state(&state_before, &transition.state).ok())
            .unwrap_or_else(|| state_before.clone());
        let pending_timers = outcome
            .as_ref()
            .ok()
            .map(|transition| {
                apply_timer_effects(
                    &timer_map,
                    &transition.timers,
                    now,
                    self.active_logic_revision(),
                )
                .values()
                .cloned()
                .collect()
            })
            .unwrap_or_else(|| timer_map.values().cloned().collect());
        Ok(Execution::with_now(
            self.active_logic_revision(),
            trigger_kind,
            snapshots,
            state_before,
            state_after,
            pending_timers,
            outcome,
            &default_site(),
            None,
        ))
    }

    fn validate_and_build_snapshots(
        &self,
        inputs: &[SimulationInput],
    ) -> Result<Vec<InputSnapshot>, SimulationError> {
        let mut supplied: Vec<Option<&SimulationInput>> = vec![None; self.config.endpoints.len()];
        for input in inputs {
            let Some(index) = self
                .config
                .endpoints
                .iter()
                .position(|endpoint| endpoint.name == input.endpoint)
            else {
                return Err(SimulationError::UnknownEndpoint(input.endpoint.clone()));
            };
            let endpoint = &self.config.endpoints[index];
            if endpoint.direction != EndpointDirection::Input {
                return Err(SimulationError::EndpointNotInput {
                    endpoint: input.endpoint.clone(),
                    actual: endpoint.direction,
                });
            }
            if supplied[index].is_some() {
                return Err(SimulationError::DuplicateInput(input.endpoint.clone()));
            }
            validate_simulation_input(endpoint, input)?;
            supplied[index] = Some(input);
        }
        self.config
            .endpoints
            .iter()
            .enumerate()
            .filter_map(|(index, endpoint)| {
                (endpoint.direction == EndpointDirection::Input).then(|| {
                    supplied[index]
                        .ok_or_else(|| SimulationError::MissingInput(endpoint.name.clone()))
                        .map(|input| InputSnapshot {
                            endpoint: endpoint.name.clone(),
                            dpt: endpoint.dpt,
                            value: input.value,
                            valid: input.valid,
                            age_ms: input.age_ms,
                        })
                })
            })
            .collect()
    }
}

impl Default for InputState {
    fn default() -> Self {
        Self {
            value: None,
            observed_at: None,
        }
    }
}
