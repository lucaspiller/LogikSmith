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
    pub outputs: Vec<OutputEffect>,
    pub timers: Vec<TimerEffect>,
}

impl Default for Transition {
    fn default() -> Self {
        Self {
            state: BTreeMap::new(),
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
    pub logic_revision: LogicRevision,
    pub trigger: Trigger,
    pub inputs: Vec<InputSnapshot>,
    pub state_before: TransientState,
    pub state_after: TransientState,
    pub pending_timers: Vec<PendingTimer>,
    pub outcome: Result<Transition, LogicError>,
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
        pending_timers: Vec<PendingTimer>,
        outcome: Result<Transition, LogicError>,
        site: &SiteTimeConfig,
        utc_unix_ms: Option<i64>,
    ) -> Self {
        Self {
            logic_revision,
            trigger,
            inputs,
            state_before,
            state_after,
            pending_timers,
            outcome,
            time_context: TimeContext::capture(site, utc_unix_ms),
        }
    }
}

pub(crate) fn validate_state_entry(key: &str, value: &StateValue) -> Result<(), StateError> {
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

pub(crate) fn validate_state_map(state: &TransientState) -> Result<(), StateError> {
    if state.len() > MAX_STATE_ENTRIES {
        return Err(StateError::TooManyEntries {
            actual: state.len(),
            maximum: MAX_STATE_ENTRIES,
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
    if total > MAX_STATE_TOTAL_BYTES {
        return Err(StateError::TotalTooLarge {
            actual: total,
            maximum: MAX_STATE_TOTAL_BYTES,
        });
    }
    Ok(())
}

pub(crate) fn validate_pending_timers(
    timers: &[PendingTimer],
    active_revision: LogicRevision,
) -> Result<(), SimulationError> {
    let mut map = BTreeMap::new();
    for timer in timers {
        if map.insert(timer.name.clone(), timer.clone()).is_some() {
            return Err(SimulationError::DuplicateTimer(timer.name.clone()));
        }
    }
    validate_pending_timer_map(&map, active_revision)
}

pub(crate) fn validate_pending_timer_map(
    timers: &BTreeMap<TimerName, PendingTimer>,
    active_revision: LogicRevision,
) -> Result<(), SimulationError> {
    if timers.len() > MAX_PENDING_TIMERS {
        return Err(SimulationError::InvalidState(StateError::TooManyEntries {
            actual: timers.len(),
            maximum: MAX_PENDING_TIMERS,
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

pub(crate) fn merge_state(
    current: &TransientState,
    patch: &StatePatch,
) -> Result<TransientState, StateError> {
    let mut merged = current.clone();
    for (key, value) in patch {
        merged.insert(key.clone(), value.clone());
    }
    validate_state_map(&merged)?;
    Ok(merged)
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
