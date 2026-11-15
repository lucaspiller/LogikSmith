use std::{collections::BTreeMap, error::Error, fmt, ops::Deref, str::FromStr};

use crate::*;

/// The immutable transition metadata for one accepted triggering input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputTrigger {
    pub endpoint: EndpointName,
    pub value: TypedValue,
    pub previous: Option<TypedValue>,
    pub changed: bool,
    pub rising: bool,
    pub falling: bool,
}

/// The immutable value and age of one configured input at execution time.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputSnapshot {
    pub endpoint: EndpointName,
    pub dpt: Dpt,
    pub value: Option<TypedValue>,
    pub valid: bool,
    pub age_ms: Option<u64>,
}

/// A distinct timer identity. Timer names intentionally use the same lexical
/// grammar as endpoint names while remaining a separate type.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TimerName(String);

pub type TimerNameError = EndpointNameError;

impl TimerName {
    pub fn new(value: impl Into<String>) -> Result<Self, EndpointNameError> {
        let value = value.into();
        validate_endpoint_name(&value)?;
        Ok(Self(value))
    }

    pub fn parse(value: &str) -> Result<Self, EndpointNameError> {
        value.parse()
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for TimerName {
    type Err = EndpointNameError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl fmt::Display for TimerName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Bounded scalar values carried by the block's transient state.
#[derive(Clone, Debug, PartialEq)]
pub enum StateValue {
    Bool(bool),
    Integer(i64),
    Number(f64),
    String(String),
}

// NaN is rejected at every conversion boundary. Keeping Eq on this public
// value makes snapshots and execution records convenient to compare.
impl Eq for StateValue {}

pub type TransientState = BTreeMap<String, StateValue>;
pub type StatePatch = BTreeMap<String, StateValue>;
/// Durable scalar values owned by a host and exposed to Lua as `persistent`.
pub type PersistentState = BTreeMap<String, StateValue>;

/// Atomic set/delete operations returned by one Lua execution.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PersistentPatch {
    pub set: StatePatch,
    pub delete: std::collections::BTreeSet<String>,
}

impl StateValue {
    pub fn validate(&self, key: &str) -> Result<(), StateError> {
        validate_state_entry(key, self)
    }
}

pub fn validate_state(state: &TransientState) -> Result<(), StateError> {
    validate_state_map(state)
}

pub const MAX_STATE_ENTRIES: usize = 64;
pub const MAX_STATE_KEY_BYTES: usize = 64;
pub const MAX_STATE_STRING_BYTES: usize = 1024;
pub const MAX_STATE_TOTAL_BYTES: usize = 16 * 1024;
pub const MAX_PENDING_TIMERS: usize = 32;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StateError {
    TooManyEntries {
        actual: usize,
        maximum: usize,
    },
    KeyTooLarge {
        key: String,
        actual: usize,
        maximum: usize,
    },
    EmptyKey,
    StringTooLarge {
        key: String,
        actual: usize,
        maximum: usize,
    },
    TotalTooLarge {
        actual: usize,
        maximum: usize,
    },
    NonFiniteNumber {
        key: String,
    },
}

impl fmt::Display for StateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyEntries { actual, maximum } => write!(
                formatter,
                "state has {actual} entries; maximum is {maximum}"
            ),
            Self::KeyTooLarge {
                key,
                actual,
                maximum,
            } => write!(
                formatter,
                "state key {key:?} is {actual} bytes; maximum is {maximum}"
            ),
            Self::EmptyKey => formatter.write_str("state keys must not be empty"),
            Self::StringTooLarge {
                key,
                actual,
                maximum,
            } => write!(
                formatter,
                "state string {key:?} is {actual} bytes; maximum is {maximum}"
            ),
            Self::TotalTooLarge { actual, maximum } => {
                write!(formatter, "state uses {actual} bytes; maximum is {maximum}")
            }
            Self::NonFiniteNumber { key } => {
                write!(formatter, "state number {key:?} must be finite")
            }
        }
    }
}

impl Error for StateError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PersistentStateError {
    Disabled,
    UnknownBlock(BlockId),
    InvalidBlockState { block_id: BlockId, error: StateError },
    RuntimeStateTooLarge { actual: usize, maximum: usize },
}

impl fmt::Display for PersistentStateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Disabled => formatter.write_str("persistent state is disabled for this runtime"),
            Self::UnknownBlock(block_id) => {
                write!(formatter, "persistent state refers to unknown block {block_id}")
            }
            Self::InvalidBlockState { block_id, error } => {
                write!(formatter, "invalid persistent state for block {block_id}: {error}")
            }
            Self::RuntimeStateTooLarge { actual, maximum } => write!(
                formatter,
                "runtime state uses {actual} bytes; maximum is {maximum}"
            ),
        }
    }
}

impl Error for PersistentStateError {}

/// An operation committed against one named timer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TimerAction {
    Scheduled {
        after_ms: u32,
        due_at: MonotonicMs,
    },
    Replaced {
        previous_due_at: MonotonicMs,
        after_ms: u32,
        due_at: MonotonicMs,
    },
    Cancelled {
        previous_due_at: MonotonicMs,
    },
    CancelNoop,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimerEffect {
    pub name: TimerName,
    pub action: TimerAction,
}

/// A validated transition returned by one handler call.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Transition {
    pub state: StatePatch,
    pub persistent: PersistentPatch,
    pub outputs: Vec<OutputEffect>,
    pub timers: Vec<TimerEffect>,
}

impl Default for Transition {
    fn default() -> Self {
        Self {
            state: BTreeMap::new(),
            persistent: PersistentPatch::default(),
            outputs: Vec::new(),
            timers: Vec::new(),
        }
    }
}

/// Trigger for one semantic execution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Trigger {
    Input(InputTrigger),
    Timer(TimerTrigger),
    Schedule(ScheduleTrigger),
}

/// Compatibility view for callers that only handle input executions. Timer
/// and schedule callers should match [`Trigger`] explicitly.
impl Deref for Trigger {
    type Target = InputTrigger;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Input(trigger) => trigger,
            Self::Timer(_) => panic!("timer trigger does not have input fields"),
            Self::Schedule(_) => panic!("schedule trigger does not have input fields"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimerTrigger {
    pub name: TimerName,
    pub scheduled_at: MonotonicMs,
    pub due_at: MonotonicMs,
    pub fired_at: MonotonicMs,
    pub scheduled_logic_revision: LogicRevision,
}

/// A timer projected in a live or simulated snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingTimer {
    pub name: TimerName,
    pub scheduled_at: MonotonicMs,
    pub due_at: MonotonicMs,
    pub scheduled_logic_revision: LogicRevision,
}

impl PendingTimer {
    pub fn logic_revision(&self) -> LogicRevision {
        self.scheduled_logic_revision
    }
}

/// Result of a successful source activation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceActivation {
    pub logic_revision: LogicRevision,
    pub cancelled_timers: Vec<TimerName>,
    pub changed: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Execution {
    /// Runtime-assigned identity. Simulations intentionally leave this empty.
    pub id: Option<ExecutionId>,
    /// The execution that produced the signal which triggered this execution.
    pub causal_producer: Option<ExecutionId>,
    pub logic_revision: LogicRevision,
    pub trigger: Trigger,
    pub inputs: Vec<InputSnapshot>,
    pub state_before: TransientState,
    pub state_after: TransientState,
    pub persistent_state_before: PersistentState,
    pub persistent_state_after: PersistentState,
    pub pending_timers: Vec<PendingTimer>,
    pub outcome: Result<Transition, LogicError>,
    /// Signal values proposed by this execution. Live runtimes populate this
    /// after committing the transition; simulations populate it without
    /// changing any live signal state.
    pub signal_effects: Vec<SignalEffect>,
    /// Consumers whose endpoint values would change for these effects.
    pub eligible_consumers: Vec<SignalEndpointId>,
    /// The frozen wall-clock context this execution captured (unavailable
    /// sentinels when no wall-clock instant was supplied).
    pub time_context: TimeContext,
}

impl Execution {
    /// Private constructor: captures `time_context` from `site` and the
    /// supplied wall-clock instant so every execution path shares one
    /// capture path.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn with_now(
        logic_revision: LogicRevision,
        trigger: Trigger,
        inputs: Vec<InputSnapshot>,
        state_before: TransientState,
        state_after: TransientState,
        persistent_state_before: PersistentState,
        persistent_state_after: PersistentState,
        pending_timers: Vec<PendingTimer>,
        outcome: Result<Transition, LogicError>,
        site: &SiteTimeConfig,
        utc_unix_ms: Option<i64>,
    ) -> Self {
        Self {
            id: None,
            causal_producer: None,
            logic_revision,
            trigger,
            inputs,
            state_before,
            state_after,
            persistent_state_before,
            persistent_state_after,
            pending_timers,
            outcome,
            signal_effects: Vec::new(),
            eligible_consumers: Vec::new(),
            time_context: TimeContext::capture(site, utc_unix_ms),
        }
    }

    pub(crate) fn set_runtime_metadata(
        &mut self,
        id: ExecutionId,
        causal_producer: Option<ExecutionId>,
    ) {
        self.id = Some(id);
        self.causal_producer = causal_producer;
    }

    pub(crate) fn set_signal_metadata(
        &mut self,
        effects: Vec<SignalEffect>,
        eligible_consumers: Vec<SignalEndpointId>,
    ) {
        self.signal_effects = effects;
        self.eligible_consumers = eligible_consumers;
    }
}

pub(crate) fn validate_state_entry(key: &str, value: &StateValue) -> Result<(), StateError> {
    validate_state_key(key)?;
    match value {
        StateValue::String(value) if value.len() > MAX_STATE_STRING_BYTES => {
            return Err(StateError::StringTooLarge {
                key: key.to_owned(),
                actual: value.len(),
                maximum: MAX_STATE_STRING_BYTES,
            });
        }
        StateValue::Number(value) if !value.is_finite() => {
            return Err(StateError::NonFiniteNumber {
                key: key.to_owned(),
            });
        }
        _ => {}
    }
    Ok(())
}

pub(crate) fn validate_state_key(key: &str) -> Result<(), StateError> {
    if key.is_empty() {
        return Err(StateError::EmptyKey);
    }
    let key_bytes = key.len();
    if key_bytes > MAX_STATE_KEY_BYTES {
        return Err(StateError::KeyTooLarge {
            key: key.to_owned(),
            actual: key_bytes,
            maximum: MAX_STATE_KEY_BYTES,
        });
    }
    Ok(())
}

pub(crate) fn validate_state_map(state: &TransientState) -> Result<(), StateError> {
    validate_state_map_with_limits(state, &RuntimeLimits::desktop())
}

pub(crate) fn validate_state_map_with_limits(
    state: &TransientState,
    limits: &RuntimeLimits,
) -> Result<(), StateError> {
    if state.len() > limits.max_state_entries_per_block {
        return Err(StateError::TooManyEntries {
            actual: state.len(),
            maximum: limits.max_state_entries_per_block,
        });
    }
    let mut total = 0usize;
    for (key, value) in state {
        validate_state_entry(key, value)?;
        total = total.saturating_add(key.len());
        if let StateValue::String(value) = value {
            total = total.saturating_add(value.len());
        }
    }
    if total > limits.max_state_bytes_per_block {
        return Err(StateError::TotalTooLarge {
            actual: total,
            maximum: limits.max_state_bytes_per_block,
        });
    }
    Ok(())
}

pub(crate) fn validate_combined_state_with_limits(
    state: &TransientState,
    persistent: &PersistentState,
    limits: &RuntimeLimits,
) -> Result<(), StateError> {
    let entries = state.len().saturating_add(persistent.len());
    if entries > limits.max_state_entries_per_block {
        return Err(StateError::TooManyEntries {
            actual: entries,
            maximum: limits.max_state_entries_per_block,
        });
    }
    validate_state_map_contents(state)?;
    validate_state_map_contents(persistent)?;
    let bytes = state_bytes(state).saturating_add(state_bytes(persistent));
    if bytes > limits.max_state_bytes_per_block {
        return Err(StateError::TotalTooLarge {
            actual: bytes,
            maximum: limits.max_state_bytes_per_block,
        });
    }
    Ok(())
}

fn validate_state_map_contents(state: &TransientState) -> Result<(), StateError> {
    for (key, value) in state {
        validate_state_entry(key, value)?;
    }
    Ok(())
}

pub(crate) fn state_bytes(state: &TransientState) -> usize {
    state
        .iter()
        .fold(0usize, |total, (key, value)| {
            total.saturating_add(key.len()).saturating_add(match value {
                StateValue::String(value) => value.len(),
                _ => 0,
            })
        })
}

pub(crate) fn validate_pending_timers_with_limits(
    timers: &[PendingTimer],
    active_revision: LogicRevision,
    limits: &RuntimeLimits,
) -> Result<(), SimulationError> {
    let mut map = BTreeMap::new();
    for timer in timers {
        if map.insert(timer.name.clone(), timer.clone()).is_some() {
            return Err(SimulationError::DuplicateTimer(timer.name.clone()));
        }
    }
    validate_pending_timer_map_with_limits(&map, active_revision, limits)
}

pub(crate) fn validate_pending_timer_map_with_limits(
    timers: &BTreeMap<TimerName, PendingTimer>,
    active_revision: LogicRevision,
    limits: &RuntimeLimits,
) -> Result<(), SimulationError> {
    if timers.len() > limits.max_pending_timers_per_block {
        return Err(SimulationError::InvalidState(StateError::TooManyEntries {
            actual: timers.len(),
            maximum: limits.max_pending_timers_per_block,
        }));
    }
    for timer in timers.values() {
        if timer.scheduled_logic_revision != active_revision {
            return Err(SimulationError::TimerRevisionMismatch {
                timer: timer.name.clone(),
                scheduled: timer.scheduled_logic_revision,
                active: active_revision,
            });
        }
    }
    Ok(())
}

pub(crate) fn merge_state_with_limits(
    current: &TransientState,
    patch: &StatePatch,
    limits: &RuntimeLimits,
) -> Result<TransientState, StateError> {
    let mut merged = current.clone();
    for (key, value) in patch {
        merged.insert(key.clone(), value.clone());
    }
    validate_state_map_with_limits(&merged, limits)?;
    Ok(merged)
}

pub(crate) fn apply_persistent_patch(
    current: &PersistentState,
    patch: &PersistentPatch,
) -> PersistentState {
    let mut merged = current.clone();
    for key in &patch.delete {
        merged.remove(key);
    }
    merged.extend(patch.set.clone());
    merged
}

pub(crate) fn merge_combined_state_with_limits(
    state: &TransientState,
    state_patch: &StatePatch,
    persistent: &PersistentState,
    persistent_patch: &PersistentPatch,
    limits: &RuntimeLimits,
) -> Result<(TransientState, PersistentState), StateError> {
    let next_state = merge_state_with_limits(state, state_patch, limits)?;
    let next_persistent = apply_persistent_patch(persistent, persistent_patch);
    validate_combined_state_with_limits(&next_state, &next_persistent, limits)?;
    Ok((next_state, next_persistent))
}

pub(crate) fn resulting_state_after_execution(
    state: &TransientState,
    persistent: &PersistentState,
    outcome: &Result<Transition, LogicError>,
    limits: &RuntimeLimits,
) -> (TransientState, PersistentState) {
    match outcome {
        Ok(transition) => merge_combined_state_with_limits(
            state,
            &transition.state,
            persistent,
            &transition.persistent,
            limits,
        )
        .expect("validated transition state must merge"),
        Err(_) => (state.clone(), persistent.clone()),
    }
}

pub(crate) fn apply_timer_effects(
    pending: &BTreeMap<TimerName, PendingTimer>,
    effects: &[TimerEffect],
    now: MonotonicMs,
    revision: LogicRevision,
) -> BTreeMap<TimerName, PendingTimer> {
    let mut candidate = pending.clone();
    for effect in effects {
        match effect.action {
            TimerAction::Scheduled { due_at, .. } | TimerAction::Replaced { due_at, .. } => {
                candidate.insert(
                    effect.name.clone(),
                    PendingTimer {
                        name: effect.name.clone(),
                        scheduled_at: now,
                        due_at,
                        scheduled_logic_revision: revision,
                    },
                );
            }
            TimerAction::Cancelled { .. } | TimerAction::CancelNoop => {
                candidate.remove(&effect.name);
            }
        }
    }
    candidate
}
