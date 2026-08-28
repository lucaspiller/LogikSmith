//! Platform-independent event processing for LogikSmith.
//!
//! The core deals in named, typed endpoints. Hosts provide observations and
//! triggering input events, then execute the logical effects returned by the
//! active Lua program. Transport details such as KNX group addresses stay
//! outside this crate.

use std::{cell::Cell, collections::BTreeMap, error::Error, fmt, ops::Deref, rc::Rc, str::FromStr};

use mlua::{
    Function, HookTriggers, Lua, LuaOptions, MultiValue, StdLib, Table, Value as LuaValue, VmState,
};

/// Maximum UTF-8 source size accepted by the logic evaluator.
pub const MAX_LOGIC_SOURCE_BYTES: usize = 64 * 1024;
/// Maximum number of Lua VM instructions per source load and handler call.
pub const MAX_LOGIC_INSTRUCTIONS: u32 = 100_000;
/// Maximum Lua-managed memory used by one evaluation.
pub const MAX_LOGIC_MEMORY_BYTES: usize = 1024 * 1024;

const INSTRUCTION_LIMIT_MARKER: &str = "logiksmith instruction limit exceeded";

/// A validated logical endpoint name.
///
/// Names start with a lowercase ASCII letter and may then contain lowercase
/// ASCII letters, digits, `_`, `-`, or `.`.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct EndpointName(String);

impl EndpointName {
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

#[cfg(test)]
mod milestone7_tests {
    use super::*;

    fn n(value: &str) -> EndpointName {
        value.parse().unwrap()
    }
    fn t(value: &str) -> TimerName {
        value.parse().unwrap()
    }
    fn endpoint(value: &str, direction: EndpointDirection, dpt: Dpt) -> Endpoint {
        Endpoint::new(n(value), direction, dpt)
    }
    fn engine(source: &str) -> Engine {
        Engine::new(EngineConfig::new(
            vec![
                endpoint("wall_switch", EndpointDirection::Input, Dpt::BOOL),
                endpoint("test_light", EndpointDirection::Output, Dpt::BOOL),
            ],
            source,
        ))
    }
    fn event(value: bool) -> InputEvent {
        InputEvent::new(n("wall_switch"), TypedValue::bool(value))
    }

    #[test]
    fn state_and_named_timers_round_trip_and_expire() {
        let mut engine = engine(
            r#"
            function handle(event, input, meta, state)
              if event.type == "input" and event.rising then
                return { state = { count = (state.count or 0) + 1 }, outputs = { test_light = true }, timers = { dim = { after = seconds(2) }, off = { after = seconds(3) } } }
              end
              if event.type == "timer" and event.timer == "off" then
                return { state = { done = true }, outputs = { test_light = false } }
              end
            end
        "#,
        );
        engine.process_input(event(false), MonotonicMs(1)).unwrap();
        let first = engine.process_input(event(true), MonotonicMs(10)).unwrap();
        let transition = first.outcome.unwrap();
        assert_eq!(transition.state["count"], StateValue::Integer(1));
        assert_eq!(transition.timers.len(), 2);
        assert_eq!(engine.pending_timers()[0].name, t("dim"));
        assert_eq!(engine.next_timer_deadline(), Some(MonotonicMs(2010)));
        assert!(
            engine
                .process_next_due_timer(MonotonicMs(2009))
                .unwrap()
                .is_none()
        );
        let _dim = engine
            .process_next_due_timer(MonotonicMs(3010))
            .unwrap()
            .unwrap();
        let timer = engine
            .process_next_due_timer(MonotonicMs(3010))
            .unwrap()
            .unwrap();
        assert!(
            matches!(timer.trigger, Trigger::Timer(TimerTrigger { ref name, scheduled_at: MonotonicMs(10), due_at: MonotonicMs(3010), fired_at: MonotonicMs(3010), .. }) if name == &t("off"))
        );
        assert_eq!(engine.state()["done"], StateValue::Bool(true));
    }

    #[test]
    fn failed_transition_rolls_back_state_outputs_and_timers() {
        let mut engine = engine(
            r#"function handle() return { state = { value = { bad = true } }, outputs = { test_light = true }, timers = { off = { after = 2 } } } end"#,
        );
        let execution = engine.process_input(event(true), MonotonicMs(1)).unwrap();
        assert!(matches!(
            execution.outcome,
            Err(LogicError::InvalidResult { .. })
        ));
        assert!(engine.state().is_empty());
        assert!(engine.pending_timers().is_empty());
    }

    #[test]
    fn read_only_views_reject_assignment_and_pairs() {
        let mut bad_engine =
            engine(r#"function handle(event, input, meta, state) event.x = 1 end"#);
        let execution = bad_engine
            .process_input(event(true), MonotonicMs(1))
            .unwrap();
        assert!(
            matches!(execution.outcome, Err(LogicError::Runtime { message, .. }) if message.contains(READ_ONLY_ARGUMENT_MARKER))
        );
        let mut engine2 = engine(
            r#"function handle(event, input, meta, state) local seen = 0 for k,v in pairs(meta) do seen = seen + 1 end local k,v = next(meta) assert(k == "wall_switch") return { state = { seen = seen } } end"#,
        );
        let execution = engine2.process_input(event(true), MonotonicMs(1)).unwrap();
        assert!(execution.outcome.is_ok(), "{:?}", execution.outcome);
    }

    #[test]
    fn duration_helpers_accept_fractional_values() {
        let mut engine = engine(
            r#"function handle() return { timers = { off = { after = seconds(1.5) } } } end"#,
        );
        engine.process_input(event(true), MonotonicMs(4)).unwrap();
        assert_eq!(engine.pending_timers()[0].due_at, MonotonicMs(1504));
    }

    #[test]
    fn timer_replacement_cancellation_and_activation_are_atomic() {
        let mut engine = engine(
            r#"function handle() return { timers = { off = { after = 100 }, dim = { after = 200 } } } end"#,
        );
        engine.process_input(event(true), MonotonicMs(10)).unwrap();
        engine.process_input(event(false), MonotonicMs(20)).unwrap();
        assert_eq!(engine.pending_timers().len(), 2);
        let cancelled = engine
            .activate_source(r#"function handle() return nil end"#)
            .unwrap();
        assert_eq!(cancelled.cancelled_timers, vec![t("dim"), t("off")]);
        assert!(engine.pending_timers().is_empty());
        let same = engine
            .activate_source(r#"function handle() return nil end"#)
            .unwrap();
        assert!(!same.changed);
    }

    #[test]
    fn timer_simulation_does_not_mutate_live_state() {
        let mut engine = engine(
            r#"function handle(event, input, meta, state) if event.type == "input" then return { state = { count = 1 }, timers = { off = { after = 10 } } } end return { state = { count = 2 } } end"#,
        );
        engine.process_input(event(true), MonotonicMs(10)).unwrap();
        let before = engine.snapshot();
        let simulation = engine
            .simulate_timer(TimerSimulationScenario {
                timer: t("off"),
                fired_at: MonotonicMs(25),
                inputs: vec![SimulationInput {
                    endpoint: n("wall_switch"),
                    value: Some(TypedValue::bool(true)),
                    valid: true,
                    age_ms: Some(15),
                }],
                state: before.state.clone(),
                pending_timers: before.pending_timers.clone(),
            })
            .unwrap();
        assert_eq!(
            simulation.state_after["count"],
            StateValue::Integer(2),
            "{:?}",
            simulation.outcome
        );
        assert_eq!(engine.snapshot(), before);
    }
}

fn validate_endpoint_name(value: &str) -> Result<(), EndpointNameError> {
    let mut chars = value.chars();
    let first = chars.next().ok_or(EndpointNameError::Empty)?;
    if !first.is_ascii_lowercase() {
        return Err(EndpointNameError::InvalidStart(first));
    }
    for character in chars {
        if !(character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || matches!(character, '_' | '-' | '.'))
        {
            return Err(EndpointNameError::InvalidCharacter(character));
        }
    }
    Ok(())
}

impl FromStr for EndpointName {
    type Err = EndpointNameError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl fmt::Display for EndpointName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EndpointNameError {
    Empty,
    InvalidStart(char),
    InvalidCharacter(char),
}

impl fmt::Display for EndpointNameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("endpoint name must not be empty"),
            Self::InvalidStart(character) => write!(
                formatter,
                "endpoint name must start with a lowercase ASCII letter, got {character:?}"
            ),
            Self::InvalidCharacter(character) => write!(
                formatter,
                "endpoint name contains invalid character {character:?}"
            ),
        }
    }
}

impl Error for EndpointNameError {}

/// The direction in which an endpoint participates in the automation model.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum EndpointDirection {
    Input,
    Output,
}

impl fmt::Display for EndpointDirection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Input => "input",
            Self::Output => "output",
        })
    }
}

/// A declared logical endpoint.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Endpoint {
    pub name: EndpointName,
    pub direction: EndpointDirection,
    pub dpt: Dpt,
}

impl Endpoint {
    pub fn new(name: EndpointName, direction: EndpointDirection, dpt: Dpt) -> Self {
        Self {
            name,
            direction,
            dpt,
        }
    }

    pub fn input(name: EndpointName, dpt: Dpt) -> Self {
        Self::new(name, EndpointDirection::Input, dpt)
    }

    pub fn output(name: EndpointName, dpt: Dpt) -> Self {
        Self::new(name, EndpointDirection::Output, dpt)
    }
}

/// A structured KNX datapoint type identifier.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Dpt {
    pub major: u16,
    pub subtype: u16,
}

impl Dpt {
    /// DPT 1.001 switch.
    pub const BOOL: Self = Self {
        major: 1,
        subtype: 1,
    };

    /// DPT 5.001 percentage.
    pub const PERCENT: Self = Self {
        major: 5,
        subtype: 1,
    };

    /// Alias for callers that spell out the value's semantic name.
    pub const PERCENTAGE: Self = Self::PERCENT;

    pub fn new(major: u16, subtype: u16) -> Result<Self, DptError> {
        if major == 0 {
            return Err(DptError::MajorOutOfRange(major));
        }
        if subtype > 999 {
            return Err(DptError::SubtypeOutOfRange(subtype));
        }
        Ok(Self { major, subtype })
    }

    pub fn parse(value: &str) -> Result<Self, DptError> {
        value.parse()
    }

    pub const fn is_bool(self) -> bool {
        self.major == Self::BOOL.major && self.subtype == Self::BOOL.subtype
    }

    pub const fn is_percent(self) -> bool {
        self.major == Self::PERCENT.major && self.subtype == Self::PERCENT.subtype
    }

    pub const fn is_supported(self) -> bool {
        self.is_bool() || self.is_percent()
    }
}

impl FromStr for Dpt {
    type Err = DptError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (major, subtype) = value.split_once('.').ok_or(DptError::InvalidFormat)?;
        if major.is_empty()
            || subtype.len() != 3
            || !subtype.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(DptError::InvalidFormat);
        }
        let major = major.parse::<u16>().map_err(|_| DptError::InvalidFormat)?;
        let subtype = subtype
            .parse::<u16>()
            .map_err(|_| DptError::InvalidFormat)?;
        Self::new(major, subtype)
    }
}

impl fmt::Display for Dpt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{:03}", self.major, self.subtype)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DptError {
    InvalidFormat,
    MajorOutOfRange(u16),
    SubtypeOutOfRange(u16),
}

impl fmt::Display for DptError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFormat => formatter
                .write_str("DPT must be formatted as major.subtype with a three-digit subtype"),
            Self::MajorOutOfRange(value) => {
                write!(formatter, "DPT major {value} must be greater than zero")
            }
            Self::SubtypeOutOfRange(value) => {
                write!(formatter, "DPT subtype {value} must not exceed 999")
            }
        }
    }
}

impl Error for DptError {}

/// The semantic payload of a value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Value {
    Bool(bool),
    Percent(u8),
}

/// A value with its DPT identity attached.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TypedValue {
    pub dpt: Dpt,
    pub value: Value,
}

impl TypedValue {
    pub fn new(dpt: Dpt, value: Value) -> Result<Self, ValueError> {
        let typed = Self { dpt, value };
        typed.validate()?;
        Ok(typed)
    }

    pub const fn bool(value: bool) -> Self {
        Self {
            dpt: Dpt::BOOL,
            value: Value::Bool(value),
        }
    }

    pub fn percent(value: u8) -> Result<Self, ValueError> {
        Self::new(Dpt::PERCENT, Value::Percent(value))
    }

    pub fn validate(self) -> Result<(), ValueError> {
        match (self.dpt, self.value) {
            (dpt, Value::Bool(_)) if dpt.is_bool() => Ok(()),
            (dpt, Value::Percent(value)) if dpt.is_percent() && value <= 100 => Ok(()),
            (dpt, Value::Percent(value)) if dpt.is_percent() => {
                Err(ValueError::PercentOutOfRange(value))
            }
            (dpt, _) if !dpt.is_supported() => Err(ValueError::UnsupportedDpt(dpt)),
            (dpt, value) => Err(ValueError::DptValueMismatch { dpt, value }),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValueError {
    UnsupportedDpt(Dpt),
    DptValueMismatch { dpt: Dpt, value: Value },
    PercentOutOfRange(u8),
}

impl fmt::Display for ValueError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedDpt(dpt) => write!(formatter, "unsupported DPT {dpt}"),
            Self::DptValueMismatch { dpt, value } => {
                write!(formatter, "value {value:?} does not match DPT {dpt}")
            }
            Self::PercentOutOfRange(value) => {
                write!(
                    formatter,
                    "percentage value {value} must be in range 0..=100"
                )
            }
        }
    }
}

impl Error for ValueError {}

/// An input event supplied by a host or adapter. Only this explicit trigger
/// operation evaluates Lua; use [`InputObservation`] for passive updates.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputEvent {
    pub endpoint: EndpointName,
    pub value: TypedValue,
}

impl InputEvent {
    pub fn new(endpoint: EndpointName, value: TypedValue) -> Self {
        Self { endpoint, value }
    }
}

/// A value update that records a known input without executing the program.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputObservation {
    pub endpoint: EndpointName,
    pub value: TypedValue,
}

impl InputObservation {
    pub fn new(endpoint: EndpointName, value: TypedValue) -> Self {
        Self { endpoint, value }
    }
}

/// A logical output effect for the host to execute.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutputEffect {
    pub endpoint: EndpointName,
    pub value: TypedValue,
}

impl OutputEffect {
    pub fn new(endpoint: EndpointName, value: TypedValue) -> Self {
        Self { endpoint, value }
    }
}

/// Compatibility alias for the Milestone 5 output name.
pub type Effect = OutputEffect;

/// A host-provided monotonic timestamp retained as a small transport-neutral
/// value for core-owned timer deadlines and desktop diagnostics.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct MonotonicMs(pub u64);

impl MonotonicMs {
    pub fn checked_add(self, milliseconds: u32) -> Option<Self> {
        self.0.checked_add(u64::from(milliseconds)).map(Self)
    }
}

/// A deterministic revision derived from the exact source bytes.
///
/// This deliberately uses the same compact scalar representation as the
/// desktop's document/content revisions, while keeping revision calculation
/// inside the core's source-program boundary.
pub type LogicRevision = u64;

/// A source program and its content revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LogicProgram {
    pub source: String,
    pub revision: LogicRevision,
}

impl LogicProgram {
    /// Builds a source program without evaluating it. Configuration boundaries
    /// should use [`Self::try_new`] or [`Engine::try_new`].
    pub fn new(source: impl Into<String>) -> Self {
        let source = source.into();
        Self {
            revision: revision_for(&source),
            source,
        }
    }

    /// Builds and validates a source program in a throwaway restricted VM.
    pub fn try_new(source: impl Into<String>) -> Result<Self, LogicError> {
        let program = Self::new(source);
        validate_logic_source(&program.source)?;
        Ok(program)
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn revision(&self) -> &LogicRevision {
        &self.revision
    }
}

fn revision_for(source: &str) -> LogicRevision {
    // FNV-1a is small, deterministic across hosts, and sufficient for a
    // content revision token. The document revision remains the host's
    // stronger stale-write token.
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in source.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Stable categories for failures originating in the Lua logic program.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogicErrorKind {
    Syntax,
    Load,
    Runtime,
    InstructionLimit,
    MemoryLimit,
    InvalidResult,
}

impl LogicErrorKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Syntax => "syntax",
            Self::Load => "load",
            Self::Runtime => "runtime",
            Self::InstructionLimit => "instruction_limit",
            Self::MemoryLimit => "memory_limit",
            Self::InvalidResult => "invalid_result",
        }
    }
}

impl fmt::Display for LogicErrorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A browser-safe, typed Lua failure. `line` is best-effort because Lua
/// errors raised by user code do not always carry source location metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LogicError {
    EmptySource,
    SourceTooLarge {
        actual: usize,
        maximum: usize,
    },
    Syntax {
        message: String,
        line: Option<usize>,
    },
    Load {
        message: String,
        line: Option<usize>,
    },
    Runtime {
        message: String,
        line: Option<usize>,
    },
    InstructionLimit {
        message: String,
        line: Option<usize>,
    },
    MemoryLimit {
        message: String,
        line: Option<usize>,
    },
    InvalidResult {
        message: String,
        line: Option<usize>,
    },
}

impl LogicError {
    pub const fn kind(&self) -> LogicErrorKind {
        match self {
            Self::EmptySource | Self::SourceTooLarge { .. } | Self::Load { .. } => {
                LogicErrorKind::Load
            }
            Self::Syntax { .. } => LogicErrorKind::Syntax,
            Self::Runtime { .. } => LogicErrorKind::Runtime,
            Self::InstructionLimit { .. } => LogicErrorKind::InstructionLimit,
            Self::MemoryLimit { .. } => LogicErrorKind::MemoryLimit,
            Self::InvalidResult { .. } => LogicErrorKind::InvalidResult,
        }
    }

    pub const fn category(&self) -> &'static str {
        self.kind().as_str()
    }

    pub fn line(&self) -> Option<usize> {
        match self {
            Self::Syntax { line, .. }
            | Self::Load { line, .. }
            | Self::Runtime { line, .. }
            | Self::InstructionLimit { line, .. }
            | Self::MemoryLimit { line, .. }
            | Self::InvalidResult { line, .. } => *line,
            Self::EmptySource | Self::SourceTooLarge { .. } => None,
        }
    }

    pub fn message(&self) -> &str {
        match self {
            Self::EmptySource => "logic.source must not be empty",
            Self::SourceTooLarge { .. } => "logic.source exceeds the 64 KiB limit",
            Self::Syntax { message, .. }
            | Self::Load { message, .. }
            | Self::Runtime { message, .. }
            | Self::InstructionLimit { message, .. }
            | Self::MemoryLimit { message, .. }
            | Self::InvalidResult { message, .. } => message,
        }
    }
}

impl fmt::Display for LogicError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(line) = self.line() {
            write!(
                formatter,
                "{} (line {line}): {}",
                self.category(),
                self.message()
            )
        } else {
            write!(formatter, "{}: {}", self.category(), self.message())
        }
    }
}

impl Error for LogicError {}

/// Configuration errors found before the engine starts processing events.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConfigError {
    DuplicateEndpoint(EndpointName),
    UnsupportedEndpointDpt { endpoint: EndpointName, dpt: Dpt },
    InvalidLogic(LogicError),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateEndpoint(endpoint) => {
                write!(formatter, "duplicate endpoint name {endpoint}")
            }
            Self::UnsupportedEndpointDpt { endpoint, dpt } => {
                write!(formatter, "endpoint {endpoint} uses unsupported DPT {dpt}")
            }
            Self::InvalidLogic(error) => write!(formatter, "logic.source: {error}"),
        }
    }
}

impl Error for ConfigError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidLogic(error) => Some(error),
            _ => None,
        }
    }
}

/// Errors validating a host-supplied input event or passive observation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EventError {
    UnknownEndpoint(EndpointName),
    EndpointNotInput {
        endpoint: EndpointName,
        actual: EndpointDirection,
    },
    DptMismatch {
        endpoint: EndpointName,
        expected: Dpt,
        actual: Dpt,
    },
    InvalidValue(ValueError),
    TimeWentBackwards {
        previous: MonotonicMs,
        current: MonotonicMs,
    },
    StaleTimer {
        timer: TimerName,
        scheduled_logic_revision: LogicRevision,
        active_logic_revision: LogicRevision,
    },
}

impl fmt::Display for EventError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownEndpoint(endpoint) => {
                write!(formatter, "unknown input endpoint {endpoint}")
            }
            Self::EndpointNotInput { endpoint, actual } => {
                write!(formatter, "endpoint {endpoint} is {actual}, not an input")
            }
            Self::DptMismatch {
                endpoint,
                expected,
                actual,
            } => write!(
                formatter,
                "input endpoint {endpoint} expects DPT {expected}, got {actual}"
            ),
            Self::InvalidValue(error) => error.fmt(formatter),
            Self::TimeWentBackwards { previous, current } => write!(
                formatter,
                "event time {current:?} is earlier than the last accepted time {previous:?}"
            ),
            Self::StaleTimer {
                timer,
                scheduled_logic_revision,
                active_logic_revision,
            } => write!(
                formatter,
                "timer {timer} belongs to stale logic revision {scheduled_logic_revision}; active revision is {active_logic_revision}"
            ),
        }
    }
}

impl Error for EventError {}

/// The triggering input supplied for a simulation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SimulationTrigger {
    pub endpoint: EndpointName,
    pub value: TypedValue,
    pub previous: Option<TypedValue>,
}

/// One complete input value supplied for a simulation.
///
/// A valid input must include both a typed value and its age. An invalid input
/// is unknown and therefore includes neither.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SimulationInput {
    pub endpoint: EndpointName,
    pub value: Option<TypedValue>,
    pub valid: bool,
    pub age_ms: Option<u64>,
}

/// A complete, immutable input scenario for one simulated execution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SimulationScenario {
    pub trigger: SimulationTrigger,
    pub inputs: Vec<SimulationInput>,
}

/// A pending timer supplied to a timer simulation. The value is deliberately
/// a plain snapshot so simulation cannot borrow or mutate the live engine.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimerSimulationScenario {
    pub timer: TimerName,
    pub fired_at: MonotonicMs,
    pub inputs: Vec<SimulationInput>,
    pub state: TransientState,
    pub pending_timers: Vec<PendingTimer>,
}

/// Errors caused by a malformed browser-supplied simulation scenario.
///
/// Lua failures are deliberately not represented here: they are contained in
/// [`Execution::outcome`], just as they are for live input events.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SimulationError {
    UnknownEndpoint(EndpointName),
    EndpointNotInput {
        endpoint: EndpointName,
        actual: EndpointDirection,
    },
    DuplicateInput(EndpointName),
    MissingInput(EndpointName),
    DptMismatch {
        endpoint: EndpointName,
        expected: Dpt,
        actual: Dpt,
    },
    InvalidValue(ValueError),
    MissingValue(EndpointName),
    UnexpectedValue(EndpointName),
    MissingAge(EndpointName),
    UnexpectedAge(EndpointName),
    TriggerValueMismatch {
        endpoint: EndpointName,
        expected: TypedValue,
        actual: TypedValue,
    },
    TriggerAgeMismatch {
        endpoint: EndpointName,
        actual: Option<u64>,
    },
    UnknownTimer(TimerName),
    DuplicateTimer(TimerName),
    TimerRevisionMismatch {
        timer: TimerName,
        scheduled: LogicRevision,
        active: LogicRevision,
    },
    InvalidState(StateError),
    TimeWentBackwards {
        previous: MonotonicMs,
        current: MonotonicMs,
    },
}

impl fmt::Display for SimulationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownEndpoint(endpoint) => {
                write!(formatter, "unknown input endpoint {endpoint}")
            }
            Self::EndpointNotInput { endpoint, actual } => {
                write!(formatter, "endpoint {endpoint} is {actual}, not an input")
            }
            Self::DuplicateInput(endpoint) => {
                write!(
                    formatter,
                    "simulation input {endpoint} was supplied more than once"
                )
            }
            Self::MissingInput(endpoint) => {
                write!(formatter, "simulation input {endpoint} was not supplied")
            }
            Self::DptMismatch {
                endpoint,
                expected,
                actual,
            } => write!(
                formatter,
                "input endpoint {endpoint} expects DPT {expected}, got {actual}"
            ),
            Self::InvalidValue(error) => error.fmt(formatter),
            Self::MissingValue(endpoint) => {
                write!(
                    formatter,
                    "valid simulation input {endpoint} is missing its value"
                )
            }
            Self::UnexpectedValue(endpoint) => write!(
                formatter,
                "invalid simulation input {endpoint} must not include a value"
            ),
            Self::MissingAge(endpoint) => {
                write!(
                    formatter,
                    "valid simulation input {endpoint} is missing its age"
                )
            }
            Self::UnexpectedAge(endpoint) => write!(
                formatter,
                "invalid simulation input {endpoint} must not include an age"
            ),
            Self::TriggerValueMismatch {
                endpoint,
                expected,
                actual,
            } => write!(
                formatter,
                "simulation trigger {endpoint} value {actual:?} does not match its input snapshot {expected:?}"
            ),
            Self::TriggerAgeMismatch { endpoint, actual } => write!(
                formatter,
                "simulation trigger {endpoint} must have age 0, got {actual:?}"
            ),
            Self::UnknownTimer(timer) => write!(formatter, "unknown simulation timer {timer}"),
            Self::DuplicateTimer(timer) => write!(
                formatter,
                "simulation timer {timer} was supplied more than once"
            ),
            Self::TimerRevisionMismatch {
                timer,
                scheduled,
                active,
            } => write!(
                formatter,
                "simulation timer {timer} has revision {scheduled}, expected active revision {active}"
            ),
            Self::InvalidState(error) => error.fmt(formatter),
            Self::TimeWentBackwards { previous, current } => write!(
                formatter,
                "simulation time {current:?} is earlier than {previous:?}"
            ),
        }
    }
}

impl Error for SimulationError {}

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

impl Transition {
    pub fn is_empty(&self) -> bool {
        self.state.is_empty() && self.outputs.is_empty() && self.timers.is_empty()
    }
}

/// Trigger for one semantic execution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Trigger {
    Input(InputTrigger),
    Timer(TimerTrigger),
}

/// Compatibility view for callers that only handle input executions. Timer
/// callers should match [`Trigger`] explicitly.
impl Deref for Trigger {
    type Target = InputTrigger;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Input(trigger) => trigger,
            Self::Timer(_) => panic!("timer trigger does not have input fields"),
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

/// One complete semantic execution, including contained Lua failures.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Execution {
    pub logic_revision: LogicRevision,
    pub trigger: Trigger,
    pub inputs: Vec<InputSnapshot>,
    pub state_before: TransientState,
    pub state_after: TransientState,
    pub pending_timers: Vec<PendingTimer>,
    pub outcome: Result<Transition, LogicError>,
}

/// A source-backed portable endpoint engine.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EngineConfig {
    pub endpoints: Vec<Endpoint>,
    pub logic: LogicProgram,
}

impl EngineConfig {
    pub fn new(endpoints: Vec<Endpoint>, source: impl Into<String>) -> Self {
        Self {
            endpoints,
            logic: LogicProgram::new(source),
        }
    }

    pub fn with_program(endpoints: Vec<Endpoint>, logic: LogicProgram) -> Self {
        Self { endpoints, logic }
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        for (index, endpoint) in self.endpoints.iter().enumerate() {
            if self
                .endpoints
                .iter()
                .take(index)
                .any(|other| other.name == endpoint.name)
            {
                return Err(ConfigError::DuplicateEndpoint(endpoint.name.clone()));
            }
            if !endpoint.dpt.is_supported() {
                return Err(ConfigError::UnsupportedEndpointDpt {
                    endpoint: endpoint.name.clone(),
                    dpt: endpoint.dpt,
                });
            }
        }
        let expected_revision = revision_for(&self.logic.source);
        if self.logic.revision != expected_revision {
            return Err(ConfigError::InvalidLogic(LogicError::Load {
                message: "logic source revision does not match its source bytes".to_owned(),
                line: None,
            }));
        }
        validate_logic_source(&self.logic.source).map_err(ConfigError::InvalidLogic)
    }

    pub fn logic_source(&self) -> &str {
        &self.logic.source
    }
}

/// Read-only engine state useful for diagnostics and snapshot endpoints.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EngineSnapshot {
    pub logic_revision: LogicRevision,
    /// Known values in configured input declaration order. Unknown values are
    /// absent, rather than being represented as `false` or `0`.
    pub known_inputs: Vec<(EndpointName, TypedValue)>,
    pub state: TransientState,
    pub pending_timers: Vec<PendingTimer>,
}

/// The core event-to-Lua-to-effect engine.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Engine {
    config: EngineConfig,
    inputs: Vec<InputState>,
    state: TransientState,
    pending_timers: BTreeMap<TimerName, PendingTimer>,
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
    pub fn process_input(
        &mut self,
        event: InputEvent,
        now: MonotonicMs,
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
        Ok(Execution {
            logic_revision: self.active_logic_revision(),
            trigger: Trigger::Input(trigger),
            inputs: snapshots,
            state_before,
            state_after,
            pending_timers,
            outcome,
        })
    }

    /// Consumes and evaluates at most one due timer. The timer is removed
    /// before Lua is called, so failure does not cause an automatic retry.
    pub fn process_next_due_timer(
        &mut self,
        now: MonotonicMs,
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
        Ok(Some(Execution {
            logic_revision: self.active_logic_revision(),
            trigger: public_trigger,
            inputs: snapshots,
            state_before,
            state_after,
            pending_timers,
            outcome,
        }))
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
        Ok(Execution {
            logic_revision: self.active_logic_revision(),
            trigger: Trigger::Input(trigger),
            inputs: snapshots,
            state_before,
            state_after,
            pending_timers,
            outcome,
        })
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
        Ok(Execution {
            logic_revision: self.active_logic_revision(),
            trigger,
            inputs: snapshots,
            state_before,
            state_after,
            pending_timers,
            outcome,
        })
    }

    fn validate_input(
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

    fn input_snapshots(&self, now: MonotonicMs) -> Vec<InputSnapshot> {
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
        Ok(Execution {
            logic_revision: self.active_logic_revision(),
            trigger: trigger_kind,
            inputs: snapshots,
            state_before,
            state_after,
            pending_timers,
            outcome,
        })
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

fn validate_simulation_input(
    endpoint: &Endpoint,
    input: &SimulationInput,
) -> Result<(), SimulationError> {
    if input.valid {
        let value = input
            .value
            .ok_or_else(|| SimulationError::MissingValue(input.endpoint.clone()))?;
        if input.age_ms.is_none() {
            return Err(SimulationError::MissingAge(input.endpoint.clone()));
        }
        validate_simulation_value(endpoint, value)
    } else {
        if input.value.is_some() {
            return Err(SimulationError::UnexpectedValue(input.endpoint.clone()));
        }
        if input.age_ms.is_some() {
            return Err(SimulationError::UnexpectedAge(input.endpoint.clone()));
        }
        Ok(())
    }
}

fn validate_simulation_value(
    endpoint: &Endpoint,
    value: TypedValue,
) -> Result<(), SimulationError> {
    value.validate().map_err(SimulationError::InvalidValue)?;
    if endpoint.dpt != value.dpt {
        return Err(SimulationError::DptMismatch {
            endpoint: endpoint.name.clone(),
            expected: endpoint.dpt,
            actual: value.dpt,
        });
    }
    Ok(())
}

fn input_trigger(
    endpoint: EndpointName,
    value: TypedValue,
    previous: Option<TypedValue>,
) -> InputTrigger {
    InputTrigger {
        endpoint,
        value,
        previous,
        changed: previous.is_some_and(|previous| previous != value),
        rising: matches!(
            (previous.map(|previous| previous.value), value.value),
            (Some(Value::Bool(false)), Value::Bool(true))
        ),
        falling: matches!(
            (previous.map(|previous| previous.value), value.value),
            (Some(Value::Bool(true)), Value::Bool(false))
        ),
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

#[derive(Clone, Copy)]
enum LuaPhase {
    Syntax,
    Load,
    Runtime,
    InvalidResult,
}

fn check_source_size(source: &str) -> Result<(), LogicError> {
    if source.trim().is_empty() {
        return Err(LogicError::EmptySource);
    }
    if source.len() > MAX_LOGIC_SOURCE_BYTES {
        return Err(LogicError::SourceTooLarge {
            actual: source.len(),
            maximum: MAX_LOGIC_SOURCE_BYTES,
        });
    }
    Ok(())
}

fn new_lua() -> Result<Lua, mlua::Error> {
    // Base functions are always initialized by Lua, but the explicit library
    // list ensures package/io/os/debug/coroutine are never loaded at all.
    let lua = Lua::new_with(
        StdLib::MATH | StdLib::STRING | StdLib::TABLE | StdLib::UTF8,
        LuaOptions::default(),
    )?;
    lua.set_memory_limit(MAX_LOGIC_MEMORY_BYTES)?;
    Ok(lua)
}

fn restricted_environment(lua: &Lua) -> Result<Table, mlua::Error> {
    let globals = lua.globals();
    let environment = lua.create_table()?;

    // Keep this allowlist intentionally small. In particular, `_G`, load,
    // dofile, loadfile, require, print, and collectgarbage are not copied into
    // the per-execution environment.
    const SAFE_BASE: &[&str] = &[
        "assert", "error", "ipairs", "pairs", "select", "tonumber", "tostring", "type",
    ];
    for name in SAFE_BASE {
        let value: LuaValue = globals.get(*name)?;
        environment.set(*name, value)?;
    }
    for name in ["math", "string", "table", "utf8"] {
        let value: LuaValue = globals.get(name)?;
        environment.set(name, value)?;
    }
    let builtin_next: Function = globals.get("next")?;
    let safe_next = lua.create_function(
        move |_lua, (table, key): (LuaValue, LuaValue)| -> Result<MultiValue, mlua::Error> {
            if let LuaValue::Table(table) = &table
                && let Some(metatable) = table.metatable()
                && metatable
                    .get::<bool>("__logiksmith_readonly")
                    .unwrap_or(false)
            {
                let next_fn: Function = metatable.get("__logiksmith_next")?;
                return next_fn.call((key,));
            }
            builtin_next.call((table, key))
        },
    )?;
    environment.set("next", safe_next)?;
    environment.set("seconds", duration_helper(lua, 1_000)?)?;
    environment.set("minutes", duration_helper(lua, 60_000)?)?;
    environment.set("hours", duration_helper(lua, 3_600_000)?)?;
    environment.set("days", duration_helper(lua, 86_400_000)?)?;
    Ok(environment)
}

const READ_ONLY_ARGUMENT_MARKER: &str = "logiksmith read-only argument";

fn duration_helper(lua: &Lua, factor: u64) -> Result<Function, mlua::Error> {
    lua.create_function(move |_lua, value: LuaValue| {
        let number = match value {
            LuaValue::Integer(value) => value as f64,
            LuaValue::Number(value) => value,
            other => {
                return Err(mlua::Error::RuntimeError(format!(
                    "duration helper expects a positive finite number, got {}",
                    other.type_name()
                )));
            }
        };
        if !number.is_finite() || number <= 0.0 {
            return Err(mlua::Error::RuntimeError(
                "duration helper expects a positive finite number".to_owned(),
            ));
        }
        let milliseconds = number * factor as f64;
        if !milliseconds.is_finite()
            || milliseconds < 1.0
            || milliseconds.fract() != 0.0
            || milliseconds > u32::MAX as f64
        {
            return Err(mlua::Error::RuntimeError(
                "duration helper result must be a whole millisecond in range 1..=u32::MAX"
                    .to_owned(),
            ));
        }
        Ok(milliseconds as i64)
    })
}

fn readonly_proxy(lua: &Lua, backing: Table) -> Result<Table, mlua::Error> {
    let proxy = lua.create_table()?;
    let metatable = lua.create_table()?;
    metatable.set("__logiksmith_readonly", true)?;
    metatable.set("__index", backing.clone())?;
    metatable.set(
        "__newindex",
        lua.create_function(
            |_, (_table, _key, _value): (LuaValue, LuaValue, LuaValue)| {
                Err::<(), _>(mlua::Error::RuntimeError(
                    READ_ONLY_ARGUMENT_MARKER.to_owned(),
                ))
            },
        )?,
    )?;

    let pairs_backing = backing.clone();
    metatable.set(
        "__pairs",
        lua.create_function(move |lua, ()| {
            let entries = pairs_backing
                .pairs::<LuaValue, LuaValue>()
                .collect::<Result<Vec<_>, _>>()?;
            let index = Rc::new(Cell::new(0usize));
            let iterator =
                lua.create_function(move |_lua, (_state, _key): (LuaValue, LuaValue)| {
                    let current = index.get();
                    if current >= entries.len() {
                        return Ok(MultiValue::from_vec(vec![LuaValue::Nil]));
                    }
                    index.set(current + 1);
                    Ok(MultiValue::from_vec(vec![
                        entries[current].0.clone(),
                        entries[current].1.clone(),
                    ]))
                })?;
            Ok((iterator, LuaValue::Nil, LuaValue::Nil))
        })?,
    )?;

    let next_backing = backing;
    metatable.set(
        "__logiksmith_next",
        lua.create_function(move |_lua, key: LuaValue| {
            let entries = next_backing
                .pairs::<LuaValue, LuaValue>()
                .collect::<Result<Vec<_>, _>>()?;
            let index = if key.is_nil() {
                Some(0)
            } else {
                entries
                    .iter()
                    .position(|(entry_key, _)| entry_key.equals(&key).unwrap_or(false))
                    .map(|position| position + 1)
            };
            let Some(index) = index else {
                return Err(mlua::Error::RuntimeError(
                    "invalid key to 'next'".to_owned(),
                ));
            };
            if index >= entries.len() {
                Ok(MultiValue::from_vec(vec![LuaValue::Nil]))
            } else {
                Ok(MultiValue::from_vec(vec![
                    entries[index].0.clone(),
                    entries[index].1.clone(),
                ]))
            }
        })?,
    )?;
    proxy.set_metatable(Some(metatable));
    Ok(proxy)
}

fn install_instruction_hook(lua: &Lua) {
    let count = Rc::new(Cell::new(0_u32));
    lua.set_hook(
        HookTriggers::new().every_nth_instruction(1),
        move |_lua, _debug| {
            let next = count.get().saturating_add(1);
            count.set(next);
            if next >= MAX_LOGIC_INSTRUCTIONS {
                Err(mlua::Error::RuntimeError(
                    INSTRUCTION_LIMIT_MARKER.to_owned(),
                ))
            } else {
                Ok(VmState::Continue)
            }
        },
    );
}

fn validate_logic_source(source: &str) -> Result<(), LogicError> {
    check_source_size(source)?;
    let lua = new_lua().map_err(|error| map_lua_error(error, LuaPhase::Load))?;
    let environment =
        restricted_environment(&lua).map_err(|error| map_lua_error(error, LuaPhase::Load))?;
    install_instruction_hook(&lua);

    let chunk = lua
        .load(source)
        .set_name("logic.source")
        .set_environment(environment.clone())
        .into_function()
        .map_err(|error| map_lua_error(error, LuaPhase::Syntax))?;
    chunk
        .call::<()>(())
        .map_err(|error| map_lua_error(error, LuaPhase::Load))?;
    match environment
        .get::<LuaValue>("handle")
        .map_err(|error| map_lua_error(error, LuaPhase::Load))?
    {
        LuaValue::Function(_) => Ok(()),
        value => Err(LogicError::Load {
            message: format!(
                "logic.source must define callable handle, got {}",
                value.type_name()
            ),
            line: None,
        }),
    }
}

fn execute_logic(
    endpoints: &[Endpoint],
    program: &LogicProgram,
    snapshots: &[InputSnapshot],
    trigger: &Trigger,
    state: &TransientState,
    pending_timers: &BTreeMap<TimerName, PendingTimer>,
    now: MonotonicMs,
) -> Result<Transition, LogicError> {
    // Validate size even though an active program was previously checked. It
    // keeps this boundary correct if a LogicProgram is constructed directly.
    check_source_size(&program.source)?;
    let lua = new_lua().map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
    let environment =
        restricted_environment(&lua).map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
    install_instruction_hook(&lua);

    let chunk = lua
        .load(program.source.as_str())
        .set_name("logic.source")
        .set_environment(environment.clone())
        .into_function()
        .map_err(|error| map_lua_error(error, LuaPhase::Syntax))?;
    chunk
        .call::<()>(())
        .map_err(|error| map_lua_error(error, LuaPhase::Load))?;
    let handle = match environment
        .get::<LuaValue>("handle")
        .map_err(|error| map_lua_error(error, LuaPhase::Load))?
    {
        LuaValue::Function(function) => function,
        value => {
            return Err(LogicError::Load {
                message: format!(
                    "logic.source must define callable handle, got {}",
                    value.type_name()
                ),
                line: None,
            });
        }
    };

    let event_backing = lua
        .create_table()
        .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
    event_backing
        .set("type", "input")
        .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
    match trigger {
        Trigger::Input(trigger) => {
            event_backing
                .set("input", trigger.endpoint.as_str())
                .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
            event_backing
                .set(
                    "value",
                    typed_value_to_lua(trigger.value).map_err(|message| LogicError::Runtime {
                        message,
                        line: None,
                    })?,
                )
                .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
            event_backing
                .set(
                    "previous",
                    trigger
                        .previous
                        .map(typed_value_to_lua)
                        .transpose()
                        .map_err(|message| LogicError::Runtime {
                            message,
                            line: None,
                        })?
                        .unwrap_or(LuaValue::Nil),
                )
                .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
            event_backing
                .set("changed", trigger.changed)
                .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
            event_backing
                .set("rising", trigger.rising)
                .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
            event_backing
                .set("falling", trigger.falling)
                .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
        }
        Trigger::Timer(trigger) => {
            event_backing
                .set("type", "timer")
                .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
            event_backing
                .set("timer", trigger.name.as_str())
                .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
            event_backing
                .set("scheduled_at", trigger.scheduled_at.0)
                .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
            event_backing
                .set("due_at", trigger.due_at.0)
                .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
            event_backing
                .set("fired_at", trigger.fired_at.0)
                .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
        }
    }

    let input_backing = lua
        .create_table()
        .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
    let meta_backing = lua
        .create_table()
        .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
    for snapshot in snapshots {
        if let Some(value) = snapshot.value {
            input_backing
                .set(
                    snapshot.endpoint.as_str(),
                    typed_value_to_lua(value).map_err(|message| LogicError::Runtime {
                        message,
                        line: None,
                    })?,
                )
                .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
        }
        let metadata_backing = lua
            .create_table()
            .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
        metadata_backing
            .set("valid", snapshot.valid)
            .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
        if let Some(age_ms) = snapshot.age_ms {
            metadata_backing
                .set("age_ms", age_ms)
                .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
        }
        let metadata = readonly_proxy(&lua, metadata_backing)
            .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
        meta_backing
            .set(snapshot.endpoint.as_str(), metadata)
            .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
    }

    let state_backing = lua
        .create_table()
        .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
    for (key, value) in state {
        state_backing
            .set(
                key.as_str(),
                state_value_to_lua(&lua, value)
                    .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?,
            )
            .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
    }
    let event_table = readonly_proxy(&lua, event_backing)
        .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
    let input_table = readonly_proxy(&lua, input_backing)
        .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
    let meta_table = readonly_proxy(&lua, meta_backing)
        .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
    let state_table = readonly_proxy(&lua, state_backing)
        .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;

    let returned: MultiValue = handle
        .call((event_table, input_table, meta_table, state_table))
        .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
    let mut returned = returned.into_iter();
    let result = returned.next().unwrap_or(LuaValue::Nil);
    if returned.next().is_some() {
        return Err(LogicError::InvalidResult {
            message: "handle must return nil or one result table".to_owned(),
            line: None,
        });
    }
    convert_result(endpoints, result, state, pending_timers, now)
}

fn typed_value_to_lua(value: TypedValue) -> Result<LuaValue, String> {
    match (value.dpt, value.value) {
        (dpt, Value::Bool(value)) if dpt.is_bool() => Ok(LuaValue::Boolean(value)),
        (dpt, Value::Percent(value)) if dpt.is_percent() && value <= 100 => {
            Ok(LuaValue::Integer(i64::from(value)))
        }
        _ => Err(format!("invalid typed value {value:?}")),
    }
}

fn state_value_to_lua(lua: &Lua, value: &StateValue) -> Result<LuaValue, mlua::Error> {
    match value {
        StateValue::Bool(value) => Ok(LuaValue::Boolean(*value)),
        StateValue::Integer(value) => Ok(LuaValue::Integer(*value)),
        StateValue::Number(value) => Ok(LuaValue::Number(*value)),
        StateValue::String(value) => Ok(LuaValue::String(lua.create_string(value)?)),
    }
}

fn convert_state_value(key: &str, value: LuaValue) -> Result<StateValue, LogicError> {
    let state_value = match value {
        LuaValue::Boolean(value) => StateValue::Bool(value),
        LuaValue::Integer(value) => StateValue::Integer(value),
        LuaValue::Number(value) if value.is_finite() => StateValue::Number(value),
        LuaValue::Number(_) => {
            return Err(LogicError::InvalidResult {
                message: format!("state value {key:?} must be finite"),
                line: None,
            });
        }
        LuaValue::String(value) => StateValue::String(
            value
                .to_str()
                .map_err(|error| LogicError::InvalidResult {
                    message: format!("state string {key:?} is not valid UTF-8: {error}"),
                    line: None,
                })?
                .to_owned(),
        ),
        value => {
            return Err(LogicError::InvalidResult {
                message: format!(
                    "state value {key:?} must be boolean, integer, finite number, or string, got {}",
                    value.type_name()
                ),
                line: None,
            });
        }
    };
    validate_state_entry(key, &state_value).map_err(|error| LogicError::InvalidResult {
        message: error.to_string(),
        line: None,
    })?;
    Ok(state_value)
}

fn convert_result(
    endpoints: &[Endpoint],
    result: LuaValue,
    current_state: &TransientState,
    pending_timers: &BTreeMap<TimerName, PendingTimer>,
    now: MonotonicMs,
) -> Result<Transition, LogicError> {
    let result_table = match result {
        LuaValue::Nil => {
            return Ok(Transition {
                state: BTreeMap::new(),
                outputs: Vec::new(),
                timers: Vec::new(),
            });
        }
        LuaValue::Table(table) => table,
        value => {
            return Err(LogicError::InvalidResult {
                message: format!(
                    "handle result must be nil or a table, got {}",
                    value.type_name()
                ),
                line: None,
            });
        }
    };

    let mut state: Option<Table> = None;
    let mut outputs: Option<Table> = None;
    let mut timers: Option<Table> = None;
    for pair in result_table.pairs::<LuaValue, LuaValue>() {
        let (key, value) = pair.map_err(|error| map_lua_error(error, LuaPhase::InvalidResult))?;
        let key = match key {
            LuaValue::String(key) => key
                .to_str()
                .map_err(|error| map_lua_error(error, LuaPhase::InvalidResult))?
                .to_owned(),
            value => {
                return Err(LogicError::InvalidResult {
                    message: format!("unsupported result field key of type {}", value.type_name()),
                    line: None,
                });
            }
        };
        if !matches!(key.as_str(), "state" | "outputs" | "timers") {
            return Err(LogicError::InvalidResult {
                message: format!(
                    "unsupported result field {key:?}; only state, outputs, and timers are allowed"
                ),
                line: None,
            });
        }
        match value {
            LuaValue::Table(table) if key == "state" && state.is_none() => state = Some(table),
            LuaValue::Table(table) if key == "outputs" && outputs.is_none() => {
                outputs = Some(table)
            }
            LuaValue::Table(table) if key == "timers" && timers.is_none() => timers = Some(table),
            LuaValue::Table(_) => {
                return Err(LogicError::InvalidResult {
                    message: format!("result contains duplicate {key} fields"),
                    line: None,
                });
            }
            value => {
                return Err(LogicError::InvalidResult {
                    message: format!("{key} must be a table, got {}", value.type_name()),
                    line: None,
                });
            }
        }
    }
    let mut state_patch = BTreeMap::new();
    if let Some(state_table) = state {
        for pair in state_table.pairs::<LuaValue, LuaValue>() {
            let (key, value) =
                pair.map_err(|error| map_lua_error(error, LuaPhase::InvalidResult))?;
            let key = match key {
                LuaValue::String(key) => key
                    .to_str()
                    .map_err(|error| map_lua_error(error, LuaPhase::InvalidResult))?
                    .to_owned(),
                value => {
                    return Err(LogicError::InvalidResult {
                        message: format!("state key must be a string, got {}", value.type_name()),
                        line: None,
                    });
                }
            };
            let value = convert_state_value(&key, value)?;
            if state_patch.insert(key.clone(), value).is_some() {
                return Err(LogicError::InvalidResult {
                    message: format!("state key {key:?} was returned more than once"),
                    line: None,
                });
            }
        }
    }
    merge_state(current_state, &state_patch).map_err(|error| LogicError::InvalidResult {
        message: error.to_string(),
        line: None,
    })?;

    let Some(outputs_table) = outputs else {
        let timers = convert_timers(timers, pending_timers, now)?;
        return Ok(Transition {
            state: state_patch,
            outputs: Vec::new(),
            timers,
        });
    };

    // Keep temporary slots until every returned field has passed validation;
    // this is the all-or-nothing boundary before any host write is possible.
    let mut values: Vec<Option<TypedValue>> = vec![None; endpoints.len()];
    for pair in outputs_table.pairs::<LuaValue, LuaValue>() {
        let (key, value) = pair.map_err(|error| map_lua_error(error, LuaPhase::InvalidResult))?;
        let key = match key {
            LuaValue::String(key) => key
                .to_str()
                .map_err(|error| map_lua_error(error, LuaPhase::InvalidResult))?
                .to_owned(),
            value => {
                return Err(LogicError::InvalidResult {
                    message: format!("output name must be a string, got {}", value.type_name()),
                    line: None,
                });
            }
        };
        let (index, endpoint) = endpoints
            .iter()
            .enumerate()
            .find(|(_, endpoint)| {
                endpoint.direction == EndpointDirection::Output && endpoint.name.as_str() == key
            })
            .ok_or_else(|| LogicError::InvalidResult {
                message: format!("unknown output endpoint {key}"),
                line: None,
            })?;
        let typed = lua_to_typed_value(endpoint, value)?;
        if values[index].replace(typed).is_some() {
            return Err(LogicError::InvalidResult {
                message: format!("output endpoint {key} was returned more than once"),
                line: None,
            });
        }
    }

    let outputs = endpoints
        .iter()
        .enumerate()
        .filter_map(|(index, endpoint)| {
            (endpoint.direction == EndpointDirection::Output)
                .then(|| {
                    values[index].map(|value| OutputEffect {
                        endpoint: endpoint.name.clone(),
                        value,
                    })
                })
                .flatten()
        })
        .collect::<Vec<_>>();
    let timers = convert_timers(timers, pending_timers, now)?;
    Ok(Transition {
        state: state_patch,
        outputs,
        timers,
    })
}

fn convert_timers(
    timers: Option<Table>,
    pending: &BTreeMap<TimerName, PendingTimer>,
    now: MonotonicMs,
) -> Result<Vec<TimerEffect>, LogicError> {
    let Some(timers) = timers else {
        return Ok(Vec::new());
    };
    let mut raw = Vec::new();
    for pair in timers.pairs::<LuaValue, LuaValue>() {
        let (key, value) = pair.map_err(|error| map_lua_error(error, LuaPhase::InvalidResult))?;
        let key = match key {
            LuaValue::String(key) => key
                .to_str()
                .map_err(|error| map_lua_error(error, LuaPhase::InvalidResult))?
                .to_owned(),
            value => {
                return Err(LogicError::InvalidResult {
                    message: format!("timer name must be a string, got {}", value.type_name()),
                    line: None,
                });
            }
        };
        let name = TimerName::new(key.clone()).map_err(|error| LogicError::InvalidResult {
            message: format!("invalid timer name {key:?}: {error}"),
            line: None,
        })?;
        let action = match value {
            LuaValue::Boolean(false) => match pending.get(&name) {
                Some(timer) => TimerAction::Cancelled {
                    previous_due_at: timer.due_at,
                },
                None => TimerAction::CancelNoop,
            },
            LuaValue::Table(schedule) => {
                let mut after: Option<u32> = None;
                for pair in schedule.pairs::<LuaValue, LuaValue>() {
                    let (field, value) =
                        pair.map_err(|error| map_lua_error(error, LuaPhase::InvalidResult))?;
                    let field = match field {
                        LuaValue::String(field) => field
                            .to_str()
                            .map_err(|error| map_lua_error(error, LuaPhase::InvalidResult))?
                            .to_owned(),
                        value => {
                            return Err(LogicError::InvalidResult {
                                message: format!(
                                    "timer {name} field must be a string, got {}",
                                    value.type_name()
                                ),
                                line: None,
                            });
                        }
                    };
                    if field != "after" {
                        return Err(LogicError::InvalidResult {
                            message: format!(
                                "timer {name} schedule only accepts after, got {field:?}"
                            ),
                            line: None,
                        });
                    }
                    let after_value = lua_duration_ms(value)?;
                    if after.replace(after_value).is_some() {
                        return Err(LogicError::InvalidResult {
                            message: format!("timer {name} contains duplicate after fields"),
                            line: None,
                        });
                    }
                }
                let after_ms = after.ok_or_else(|| LogicError::InvalidResult {
                    message: format!("timer {name} schedule requires after"),
                    line: None,
                })?;
                let due_at =
                    now.checked_add(after_ms)
                        .ok_or_else(|| LogicError::InvalidResult {
                            message: format!("timer {name} deadline overflows MonotonicMs"),
                            line: None,
                        })?;
                match pending.get(&name) {
                    Some(timer) => TimerAction::Replaced {
                        previous_due_at: timer.due_at,
                        after_ms,
                        due_at,
                    },
                    None => TimerAction::Scheduled { after_ms, due_at },
                }
            }
            value => {
                return Err(LogicError::InvalidResult {
                    message: format!(
                        "timer {name} must be false or a schedule table, got {}",
                        value.type_name()
                    ),
                    line: None,
                });
            }
        };
        raw.push(TimerEffect { name, action });
    }
    raw.sort_by(|left, right| left.name.cmp(&right.name));
    let mut candidate = pending.clone();
    for effect in &raw {
        match effect.action {
            TimerAction::Scheduled { after_ms, due_at }
            | TimerAction::Replaced {
                after_ms, due_at, ..
            } => {
                candidate.insert(
                    effect.name.clone(),
                    PendingTimer {
                        name: effect.name.clone(),
                        scheduled_at: now,
                        due_at,
                        scheduled_logic_revision: 0,
                    },
                );
                let _ = after_ms;
            }
            TimerAction::Cancelled { .. } | TimerAction::CancelNoop => {
                candidate.remove(&effect.name);
            }
        }
    }
    if candidate.len() > MAX_PENDING_TIMERS {
        return Err(LogicError::InvalidResult {
            message: format!("pending timers exceed maximum of {MAX_PENDING_TIMERS}"),
            line: None,
        });
    }
    Ok(raw)
}

fn lua_duration_ms(value: LuaValue) -> Result<u32, LogicError> {
    let number = match value {
        LuaValue::Integer(value) if (1..=i64::from(u32::MAX)).contains(&value) => {
            return Ok(value as u32);
        }
        LuaValue::Integer(value) => value as f64,
        LuaValue::Number(value) => value,
        value => {
            return Err(LogicError::InvalidResult {
                message: format!(
                    "timer after must be a positive finite whole millisecond, got {}",
                    value.type_name()
                ),
                line: None,
            });
        }
    };
    if !number.is_finite() || number <= 0.0 || number.fract() != 0.0 || number > u32::MAX as f64 {
        return Err(LogicError::InvalidResult {
            message:
                "timer after must be a positive finite whole millisecond in range 1..=u32::MAX"
                    .to_owned(),
            line: None,
        });
    }
    Ok(number as u32)
}

fn validate_state_entry(key: &str, value: &StateValue) -> Result<(), StateError> {
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

fn validate_state_map(state: &TransientState) -> Result<(), StateError> {
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

fn validate_pending_timers(
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

fn validate_pending_timer_map(
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

fn merge_state(current: &TransientState, patch: &StatePatch) -> Result<TransientState, StateError> {
    let mut merged = current.clone();
    for (key, value) in patch {
        merged.insert(key.clone(), value.clone());
    }
    validate_state_map(&merged)?;
    Ok(merged)
}

fn apply_timer_effects(
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

fn lua_to_typed_value(endpoint: &Endpoint, value: LuaValue) -> Result<TypedValue, LogicError> {
    match endpoint.dpt {
        dpt if dpt.is_bool() => match value {
            LuaValue::Boolean(value) => Ok(TypedValue::bool(value)),
            value => Err(LogicError::InvalidResult {
                message: format!(
                    "output {} expects a boolean, got {}",
                    endpoint.name,
                    value.type_name()
                ),
                line: None,
            }),
        },
        dpt if dpt.is_percent() => match value {
            LuaValue::Integer(value) if (0..=100).contains(&value) => {
                Ok(TypedValue::percent(value as u8).expect("bounded percentage"))
            }
            LuaValue::Integer(value) => Err(LogicError::InvalidResult {
                message: format!(
                    "output {} percentage {value} must be in range 0..=100",
                    endpoint.name
                ),
                line: None,
            }),
            LuaValue::Number(value) if value.is_nan() || value.is_infinite() => {
                Err(LogicError::InvalidResult {
                    message: format!(
                        "output {} percentage must be finite, got {value}",
                        endpoint.name
                    ),
                    line: None,
                })
            }
            LuaValue::Number(value) => Err(LogicError::InvalidResult {
                message: format!(
                    "output {} percentage must be an integer Lua number, got {value}",
                    endpoint.name
                ),
                line: None,
            }),
            value => Err(LogicError::InvalidResult {
                message: format!(
                    "output {} expects an integer percentage, got {}",
                    endpoint.name,
                    value.type_name()
                ),
                line: None,
            }),
        },
        _ => Err(LogicError::InvalidResult {
            message: format!(
                "output {} uses unsupported DPT {}",
                endpoint.name, endpoint.dpt
            ),
            line: None,
        }),
    }
}

fn map_lua_error(error: mlua::Error, phase: LuaPhase) -> LogicError {
    let text = truncate_message(error.to_string());
    let line = source_line(&text);
    if contains_instruction_marker(&error) {
        return LogicError::InstructionLimit {
            message: text,
            line,
        };
    }
    if contains_memory_error(&error) {
        return LogicError::MemoryLimit {
            message: text,
            line,
        };
    }
    match phase {
        LuaPhase::Syntax => LogicError::Syntax {
            message: text,
            line,
        },
        LuaPhase::Load => LogicError::Load {
            message: text,
            line,
        },
        LuaPhase::Runtime => LogicError::Runtime {
            message: text,
            line,
        },
        LuaPhase::InvalidResult => LogicError::InvalidResult {
            message: text,
            line,
        },
    }
}

fn contains_instruction_marker(error: &mlua::Error) -> bool {
    if error.to_string().contains(INSTRUCTION_LIMIT_MARKER) {
        return true;
    }
    match error {
        mlua::Error::CallbackError { cause, .. } => contains_instruction_marker(cause),
        mlua::Error::BadArgument { cause, .. } => contains_instruction_marker(cause),
        _ => false,
    }
}

fn contains_memory_error(error: &mlua::Error) -> bool {
    if matches!(error, mlua::Error::MemoryError(_)) {
        return true;
    }
    match error {
        mlua::Error::CallbackError { cause, .. } => contains_memory_error(cause),
        mlua::Error::BadArgument { cause, .. } => contains_memory_error(cause),
        _ => false,
    }
}

fn truncate_message(message: String) -> String {
    const MAX_DIAGNOSTIC_BYTES: usize = 4096;
    if message.len() <= MAX_DIAGNOSTIC_BYTES {
        return message;
    }
    let mut end = MAX_DIAGNOSTIC_BYTES;
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &message[..end])
}

fn source_line(message: &str) -> Option<usize> {
    let bytes = message.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if !bytes[index].is_ascii_digit() {
            index += 1;
            continue;
        }
        let start = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        if index < bytes.len()
            && bytes[index] == b':'
            && let Ok(line) = message[start..index].parse::<usize>()
        {
            return Some(line);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(value: &str) -> EndpointName {
        value.parse().unwrap()
    }

    fn endpoint(value: &str, direction: EndpointDirection, dpt: Dpt) -> Endpoint {
        Endpoint::new(name(value), direction, dpt)
    }

    fn source() -> &'static str {
        "function handle(event, input)\n  if event.input == 'wall_switch' and event.value == true then\n    return { outputs = { test_light = true, dimmer_output = input.dimmer_level or 0 } }\n  end\nend"
    }

    fn config() -> EngineConfig {
        EngineConfig::new(
            vec![
                endpoint("wall_switch", EndpointDirection::Input, Dpt::BOOL),
                endpoint("dimmer_level", EndpointDirection::Input, Dpt::PERCENT),
                endpoint("unused_input", EndpointDirection::Input, Dpt::BOOL),
                endpoint("test_light", EndpointDirection::Output, Dpt::BOOL),
                endpoint("dimmer_output", EndpointDirection::Output, Dpt::PERCENT),
            ],
            source(),
        )
    }

    fn trigger(value: bool) -> InputEvent {
        InputEvent::new(name("wall_switch"), TypedValue::bool(value))
    }

    fn simulation_input(
        endpoint: &str,
        value: Option<TypedValue>,
        valid: bool,
        age_ms: Option<u64>,
    ) -> SimulationInput {
        SimulationInput {
            endpoint: name(endpoint),
            value,
            valid,
            age_ms,
        }
    }

    fn simulation_scenario(
        value: bool,
        previous: Option<bool>,
        inputs: Vec<SimulationInput>,
    ) -> SimulationScenario {
        SimulationScenario {
            trigger: SimulationTrigger {
                endpoint: name("wall_switch"),
                value: TypedValue::bool(value),
                previous: previous.map(TypedValue::bool),
            },
            inputs,
        }
    }

    fn at(now: u64) -> MonotonicMs {
        MonotonicMs(now)
    }

    fn run(engine: &mut Engine, value: bool, now: u64) -> Execution {
        engine.process_input(trigger(value), at(now)).unwrap()
    }

    fn effects(execution: &Execution) -> &Vec<Effect> {
        &execution.outcome.as_ref().unwrap().outputs
    }

    #[test]
    fn endpoint_names_and_values_are_typed() {
        assert!("wall_switch".parse::<EndpointName>().is_ok());
        assert!("Wall_switch".parse::<EndpointName>().is_err());
        assert_eq!(Dpt::BOOL.to_string(), "1.001");
        assert_eq!(Dpt::PERCENT.to_string(), "5.001");
        assert!(TypedValue::new(Dpt::BOOL, Value::Percent(42)).is_err());
        assert!(TypedValue::new(Dpt::PERCENT, Value::Percent(101)).is_err());
    }

    #[test]
    fn valid_source_loads_and_required_handler_is_checked() {
        let engine = Engine::try_new(config()).unwrap();
        assert_eq!(
            engine.active_logic_revision(),
            LogicProgram::new(source()).revision
        );
        assert!(matches!(
            Engine::validate_source("function nope() end"),
            Err(LogicError::Load { .. })
        ));
    }

    #[test]
    fn syntax_and_empty_or_oversized_sources_are_rejected() {
        assert!(matches!(
            Engine::validate_source("function handle( "),
            Err(LogicError::Syntax { line: Some(_), .. })
        ));
        assert!(matches!(
            Engine::validate_source("   "),
            Err(LogicError::EmptySource)
        ));
        let oversized = "x".repeat(MAX_LOGIC_SOURCE_BYTES + 1);
        assert!(matches!(
            Engine::validate_source(&oversized),
            Err(LogicError::SourceTooLarge { .. })
        ));
    }

    #[test]
    fn observations_update_snapshot_without_execution() {
        let mut engine = Engine::new(config());
        engine
            .observe_input(
                InputObservation::new(name("dimmer_level"), TypedValue::percent(42).unwrap()),
                MonotonicMs(10),
            )
            .unwrap();
        assert_eq!(
            engine.known_input_values(),
            vec![(name("dimmer_level"), TypedValue::percent(42).unwrap())]
        );
    }

    #[test]
    fn triggering_value_is_in_snapshot_and_outputs_are_declaration_ordered() {
        let mut engine = Engine::new(config());
        engine
            .observe_input(
                InputObservation::new(name("dimmer_level"), TypedValue::percent(42).unwrap()),
                MonotonicMs(10),
            )
            .unwrap();
        let execution = run(&mut engine, true, 20);
        assert_eq!(effects(&execution).len(), 2);
        assert_eq!(
            effects(&execution).as_slice(),
            vec![
                OutputEffect {
                    endpoint: name("test_light"),
                    value: TypedValue::bool(true),
                },
                OutputEffect {
                    endpoint: name("dimmer_output"),
                    value: TypedValue::percent(42).unwrap(),
                },
            ]
        );
        assert!(
            engine
                .known_input_values()
                .contains(&(name("wall_switch"), TypedValue::bool(true)))
        );
        assert_eq!(execution.inputs[0].age_ms, Some(0));
        assert_eq!(execution.inputs[1].age_ms, Some(10));
    }

    #[test]
    fn valid_simulation_uses_complete_ordered_snapshot_and_does_not_mutate_engine() {
        let engine = Engine::new(config());
        let before = engine.snapshot();
        let scenario = simulation_scenario(
            true,
            Some(false),
            vec![
                simulation_input("unused_input", None, false, None),
                simulation_input(
                    "dimmer_level",
                    Some(TypedValue::percent(42).unwrap()),
                    true,
                    Some(25),
                ),
                simulation_input("wall_switch", Some(TypedValue::bool(true)), true, Some(0)),
            ],
        );
        let first = engine.simulate_input(scenario.clone()).unwrap();
        let repeated = engine.simulate_input(scenario).unwrap();

        assert_eq!(first, repeated);
        assert_eq!(engine.snapshot(), before);
        assert_eq!(
            first
                .inputs
                .iter()
                .map(|input| input.endpoint.as_str())
                .collect::<Vec<_>>(),
            vec!["wall_switch", "dimmer_level", "unused_input"]
        );
        assert_eq!(first.inputs[0].age_ms, Some(0));
        assert_eq!(first.inputs[1].age_ms, Some(25));
        assert_eq!(first.inputs[2].value, None);
        assert_eq!(effects(&first).len(), 2);
    }

    #[test]
    fn simulation_derives_boolean_edges_from_optional_previous_value() {
        let engine = Engine::new(EngineConfig::new(
            vec![
                endpoint("wall_switch", EndpointDirection::Input, Dpt::BOOL),
                endpoint("enabled", EndpointDirection::Input, Dpt::BOOL),
                endpoint("test_light", EndpointDirection::Output, Dpt::BOOL),
            ],
            "function handle(event, input) return nil end",
        ));
        let inputs = |value| {
            vec![
                simulation_input("wall_switch", Some(TypedValue::bool(value)), true, Some(0)),
                simulation_input("enabled", None, false, None),
            ]
        };

        let unknown = engine
            .simulate_input(simulation_scenario(false, None, inputs(false)))
            .unwrap();
        assert!(!unknown.trigger.changed);
        assert!(!unknown.trigger.rising);
        assert!(!unknown.trigger.falling);

        let rising = engine
            .simulate_input(simulation_scenario(true, Some(false), inputs(true)))
            .unwrap();
        assert!(rising.trigger.changed);
        assert!(rising.trigger.rising);
        assert!(!rising.trigger.falling);

        let falling = engine
            .simulate_input(simulation_scenario(false, Some(true), inputs(false)))
            .unwrap();
        assert!(falling.trigger.changed);
        assert!(!falling.trigger.rising);
        assert!(falling.trigger.falling);

        let repeated = engine
            .simulate_input(simulation_scenario(true, Some(true), inputs(true)))
            .unwrap();
        assert!(!repeated.trigger.changed);
        assert!(!repeated.trigger.rising);
        assert!(!repeated.trigger.falling);
    }

    #[test]
    fn simulation_preserves_percentage_values_and_output_order() {
        let engine = Engine::new(EngineConfig::new(
            vec![
                endpoint("wall_switch", EndpointDirection::Input, Dpt::BOOL),
                endpoint("level", EndpointDirection::Input, Dpt::PERCENT),
                endpoint("test_light", EndpointDirection::Output, Dpt::BOOL),
                endpoint("dimmer_output", EndpointDirection::Output, Dpt::PERCENT),
            ],
            "function handle(event, input)\n  if event.rising then return { outputs = { dimmer_output = input.level, test_light = true } } end\nend",
        ));
        let execution = engine
            .simulate_input(simulation_scenario(
                true,
                Some(false),
                vec![
                    simulation_input(
                        "level",
                        Some(TypedValue::percent(73).unwrap()),
                        true,
                        Some(8),
                    ),
                    simulation_input("wall_switch", Some(TypedValue::bool(true)), true, Some(0)),
                ],
            ))
            .unwrap();
        assert_eq!(
            effects(&execution),
            &vec![
                OutputEffect {
                    endpoint: name("test_light"),
                    value: TypedValue::bool(true),
                },
                OutputEffect {
                    endpoint: name("dimmer_output"),
                    value: TypedValue::percent(73).unwrap(),
                },
            ]
        );
    }

    #[test]
    fn simulation_rejects_incomplete_duplicate_unknown_and_malformed_inputs() {
        let engine = Engine::new(config());
        let valid_wall =
            || simulation_input("wall_switch", Some(TypedValue::bool(true)), true, Some(0));
        let valid_dimmer = || {
            simulation_input(
                "dimmer_level",
                Some(TypedValue::percent(50).unwrap()),
                true,
                Some(1),
            )
        };
        let invalid_unused = || simulation_input("unused_input", None, false, None);

        let unknown = simulation_scenario(
            true,
            None,
            vec![
                valid_wall(),
                valid_dimmer(),
                invalid_unused(),
                simulation_input("unknown", None, false, None),
            ],
        );
        assert!(matches!(
            engine.simulate_input(unknown),
            Err(SimulationError::UnknownEndpoint(endpoint)) if endpoint == name("unknown")
        ));

        let duplicate = simulation_scenario(
            true,
            None,
            vec![valid_wall(), valid_wall(), valid_dimmer(), invalid_unused()],
        );
        assert!(matches!(
            engine.simulate_input(duplicate),
            Err(SimulationError::DuplicateInput(endpoint)) if endpoint == name("wall_switch")
        ));

        let missing = simulation_scenario(true, None, vec![valid_wall(), valid_dimmer()]);
        assert!(matches!(
            engine.simulate_input(missing),
            Err(SimulationError::MissingInput(endpoint)) if endpoint == name("unused_input")
        ));

        let missing_value = simulation_scenario(
            true,
            None,
            vec![
                simulation_input("wall_switch", None, true, Some(0)),
                valid_dimmer(),
                invalid_unused(),
            ],
        );
        assert!(matches!(
            engine.simulate_input(missing_value),
            Err(SimulationError::MissingValue(endpoint)) if endpoint == name("wall_switch")
        ));

        let unexpected_value = simulation_scenario(
            true,
            None,
            vec![
                valid_wall(),
                valid_dimmer(),
                simulation_input("unused_input", Some(TypedValue::bool(false)), false, None),
            ],
        );
        assert!(matches!(
            engine.simulate_input(unexpected_value),
            Err(SimulationError::UnexpectedValue(endpoint)) if endpoint == name("unused_input")
        ));

        let missing_age = simulation_scenario(
            true,
            None,
            vec![
                simulation_input("wall_switch", Some(TypedValue::bool(true)), true, None),
                valid_dimmer(),
                invalid_unused(),
            ],
        );
        assert!(matches!(
            engine.simulate_input(missing_age),
            Err(SimulationError::MissingAge(endpoint)) if endpoint == name("wall_switch")
        ));

        let unexpected_age = simulation_scenario(
            true,
            None,
            vec![
                valid_wall(),
                valid_dimmer(),
                simulation_input("unused_input", None, false, Some(2)),
            ],
        );
        assert!(matches!(
            engine.simulate_input(unexpected_age),
            Err(SimulationError::UnexpectedAge(endpoint)) if endpoint == name("unused_input")
        ));

        let wrong_dpt = simulation_scenario(
            true,
            None,
            vec![
                simulation_input(
                    "wall_switch",
                    Some(TypedValue::percent(20).unwrap()),
                    true,
                    Some(0),
                ),
                valid_dimmer(),
                invalid_unused(),
            ],
        );
        assert!(matches!(
            engine.simulate_input(wrong_dpt),
            Err(SimulationError::DptMismatch { endpoint, .. }) if endpoint == name("wall_switch")
        ));
    }

    #[test]
    fn simulation_rejects_invalid_trigger_contract() {
        let engine = Engine::new(config());
        let complete_inputs = |wall_value, wall_age| {
            vec![
                simulation_input(
                    "wall_switch",
                    Some(TypedValue::bool(wall_value)),
                    true,
                    wall_age,
                ),
                simulation_input(
                    "dimmer_level",
                    Some(TypedValue::percent(10).unwrap()),
                    true,
                    Some(1),
                ),
                simulation_input("unused_input", None, false, None),
            ]
        };

        let mut trigger_unknown = simulation_scenario(true, None, complete_inputs(true, Some(0)));
        trigger_unknown.trigger.endpoint = name("not_configured");
        assert!(matches!(
            engine.simulate_input(trigger_unknown),
            Err(SimulationError::UnknownEndpoint(endpoint)) if endpoint == name("not_configured")
        ));

        let mut trigger_value = simulation_scenario(true, None, complete_inputs(false, Some(0)));
        trigger_value.trigger.value = TypedValue::bool(true);
        assert!(matches!(
            engine.simulate_input(trigger_value),
            Err(SimulationError::TriggerValueMismatch { endpoint, .. }) if endpoint == name("wall_switch")
        ));

        let trigger_age = simulation_scenario(true, None, complete_inputs(true, Some(9)));
        assert!(matches!(
            engine.simulate_input(trigger_age),
            Err(SimulationError::TriggerAgeMismatch { endpoint, actual: Some(9) }) if endpoint == name("wall_switch")
        ));

        let mut previous_dpt = simulation_scenario(true, None, complete_inputs(true, Some(0)));
        previous_dpt.trigger.previous = Some(TypedValue::percent(5).unwrap());
        assert!(matches!(
            engine.simulate_input(previous_dpt),
            Err(SimulationError::DptMismatch { endpoint, .. }) if endpoint == name("wall_switch")
        ));

        let invalid_trigger = simulation_scenario(
            true,
            None,
            vec![
                simulation_input("wall_switch", None, false, None),
                simulation_input(
                    "dimmer_level",
                    Some(TypedValue::percent(10).unwrap()),
                    true,
                    Some(1),
                ),
                simulation_input("unused_input", None, false, None),
            ],
        );
        assert!(matches!(
            engine.simulate_input(invalid_trigger),
            Err(SimulationError::MissingValue(endpoint)) if endpoint == name("wall_switch")
        ));
    }

    #[test]
    fn contained_lua_failures_are_returned_as_normal_simulation_executions() {
        let engine = Engine::new(EngineConfig::new(
            vec![
                endpoint("wall_switch", EndpointDirection::Input, Dpt::BOOL),
                endpoint("test_light", EndpointDirection::Output, Dpt::BOOL),
            ],
            "function handle(event, input) error('simulated failure') end",
        ));
        let execution = engine
            .simulate_input(simulation_scenario(
                true,
                None,
                vec![simulation_input(
                    "wall_switch",
                    Some(TypedValue::bool(true)),
                    true,
                    Some(0),
                )],
            ))
            .unwrap();
        assert!(matches!(execution.outcome, Err(LogicError::Runtime { .. })));
    }

    #[test]
    fn equivalent_live_and_simulated_snapshots_produce_equivalent_effects() {
        let logic = "function handle(event, input)\n  if event.rising and input.enabled == true then return { outputs = { test_light = true } } end\nend";
        let endpoints = vec![
            endpoint("wall_switch", EndpointDirection::Input, Dpt::BOOL),
            endpoint("enabled", EndpointDirection::Input, Dpt::BOOL),
            endpoint("test_light", EndpointDirection::Output, Dpt::BOOL),
        ];
        let mut live = Engine::new(EngineConfig::new(endpoints.clone(), logic));
        live.observe_input(
            InputObservation::new(name("wall_switch"), TypedValue::bool(false)),
            at(10),
        )
        .unwrap();
        live.observe_input(
            InputObservation::new(name("enabled"), TypedValue::bool(true)),
            at(10),
        )
        .unwrap();
        let live_execution = live.process_input(trigger(true), at(20)).unwrap();

        let simulated = Engine::new(EngineConfig::new(endpoints, logic))
            .simulate_input(simulation_scenario(
                true,
                Some(false),
                vec![
                    simulation_input("wall_switch", Some(TypedValue::bool(true)), true, Some(0)),
                    simulation_input("enabled", Some(TypedValue::bool(true)), true, Some(10)),
                ],
            ))
            .unwrap();

        assert_eq!(live_execution.trigger, simulated.trigger);
        assert_eq!(live_execution.outcome, simulated.outcome);
    }

    #[test]
    fn transition_metadata_covers_first_rising_falling_and_repeated_values() {
        let mut engine = Engine::new(EngineConfig::new(
            vec![
                endpoint("wall_switch", EndpointDirection::Input, Dpt::BOOL),
                endpoint("test_light", EndpointDirection::Output, Dpt::BOOL),
            ],
            "function handle(event, input) return nil end",
        ));
        let first = run(&mut engine, true, 1);
        assert_eq!(first.trigger.previous, None);
        assert!(!first.trigger.changed);
        assert!(!first.trigger.rising);
        assert!(!first.trigger.falling);
        let repeated = run(&mut engine, true, 2);
        assert_eq!(repeated.trigger.previous, Some(TypedValue::bool(true)));
        assert!(!repeated.trigger.changed);
        assert!(!repeated.trigger.rising);
        assert!(!repeated.trigger.falling);
        let falling = run(&mut engine, false, 3);
        assert_eq!(falling.trigger.previous, Some(TypedValue::bool(true)));
        assert!(falling.trigger.changed);
        assert!(!falling.trigger.rising);
        assert!(falling.trigger.falling);
    }

    #[test]
    fn percentage_changes_set_changed_without_boolean_edges() {
        let mut engine = Engine::new(EngineConfig::new(
            vec![
                endpoint("level", EndpointDirection::Input, Dpt::PERCENT),
                endpoint("test_light", EndpointDirection::Output, Dpt::BOOL),
            ],
            "function handle(event, input) return nil end",
        ));
        let event = |value| InputEvent::new(name("level"), TypedValue::percent(value).unwrap());
        let first = engine.process_input(event(10), MonotonicMs(1)).unwrap();
        assert!(!first.trigger.changed);
        let changed = engine.process_input(event(20), MonotonicMs(2)).unwrap();
        assert!(changed.trigger.changed);
        assert!(!changed.trigger.rising);
        assert!(!changed.trigger.falling);
        let same = engine.process_input(event(20), MonotonicMs(3)).unwrap();
        assert!(!same.trigger.changed);
    }

    #[test]
    fn passive_observations_establish_baseline_and_refresh_age() {
        let mut engine = Engine::new(config());
        engine
            .observe_input(
                InputObservation::new(name("dimmer_level"), TypedValue::percent(42).unwrap()),
                MonotonicMs(10),
            )
            .unwrap();
        let first = run(&mut engine, true, 20);
        assert_eq!(first.trigger.previous, None);
        assert_eq!(first.inputs[1].age_ms, Some(10));
        engine
            .observe_input(
                InputObservation::new(name("dimmer_level"), TypedValue::percent(42).unwrap()),
                MonotonicMs(30),
            )
            .unwrap();
        let refreshed = run(&mut engine, false, 35);
        assert_eq!(refreshed.inputs[1].age_ms, Some(5));
        assert!(effects(&refreshed).is_empty());
    }

    #[test]
    fn complete_snapshot_is_ordered_and_unknown_inputs_are_invalid() {
        let mut engine = Engine::new(EngineConfig::new(
            vec![
                endpoint("wall_switch", EndpointDirection::Input, Dpt::BOOL),
                endpoint("enabled", EndpointDirection::Input, Dpt::BOOL),
                endpoint("level", EndpointDirection::Input, Dpt::PERCENT),
                endpoint("test_light", EndpointDirection::Output, Dpt::BOOL),
            ],
            "function handle(event, input, meta)\n  return { outputs = { test_light = meta.enabled.valid == false and input.wall_switch == true and meta.wall_switch.age_ms == 0 } }\nend",
        ));
        engine
            .observe_input(
                InputObservation::new(name("level"), TypedValue::percent(9).unwrap()),
                MonotonicMs(100),
            )
            .unwrap();
        let execution = run(&mut engine, true, 150);
        assert_eq!(execution.inputs.len(), 3);
        assert_eq!(execution.inputs[0].endpoint, name("wall_switch"));
        assert_eq!(execution.inputs[0].age_ms, Some(0));
        assert!(execution.inputs[0].valid);
        assert_eq!(execution.inputs[1].value, None);
        assert!(!execution.inputs[1].valid);
        assert_eq!(execution.inputs[1].age_ms, None);
        assert_eq!(execution.inputs[2].age_ms, Some(50));
        assert_eq!(effects(&execution).len(), 1);
    }

    #[test]
    fn third_lua_argument_exposes_metadata_and_two_argument_scripts_work() {
        let mut engine = Engine::new(EngineConfig::new(
            vec![
                endpoint("wall_switch", EndpointDirection::Input, Dpt::BOOL),
                endpoint("enabled", EndpointDirection::Input, Dpt::BOOL),
                endpoint("test_light", EndpointDirection::Output, Dpt::BOOL),
            ],
            "function handle(event, input, meta) return { outputs = { test_light = meta.enabled.valid and meta.enabled.age_ms == 7 and event.previous == nil } } end",
        ));
        engine
            .observe_input(
                InputObservation::new(name("enabled"), TypedValue::bool(true)),
                MonotonicMs(10),
            )
            .unwrap();
        assert_eq!(effects(&run(&mut engine, true, 17)).len(), 1);
        engine
            .replace_source(
                "function handle(event, input) return { outputs = { test_light = event.value } } end",
            )
            .unwrap();
        assert_eq!(effects(&run(&mut engine, false, 18)).len(), 1);
    }

    #[test]
    fn zero_effect_success_and_contained_lua_failure_keep_full_execution() {
        let mut engine = Engine::new(EngineConfig::new(
            vec![
                endpoint("wall_switch", EndpointDirection::Input, Dpt::BOOL),
                endpoint("test_light", EndpointDirection::Output, Dpt::BOOL),
            ],
            "function handle(event, input) return nil end",
        ));
        let success = run(&mut engine, true, 1);
        assert_eq!(
            success.outcome,
            Ok(Transition {
                state: BTreeMap::new(),
                outputs: Vec::new(),
                timers: Vec::new(),
            })
        );
        assert_eq!(success.inputs[0].value, Some(TypedValue::bool(true)));
        engine
            .replace_source("function handle(event, input) error('boom') end")
            .unwrap();
        let failed = run(&mut engine, false, 2);
        assert!(matches!(failed.outcome, Err(LogicError::Runtime { .. })));
        assert_eq!(failed.trigger.previous, Some(TypedValue::bool(true)));
        assert!(failed.inputs[0].valid);
    }

    #[test]
    fn strict_return_conversion_is_all_or_nothing() {
        let mut engine = Engine::new(EngineConfig::new(
            vec![
                endpoint("wall_switch", EndpointDirection::Input, Dpt::BOOL),
                endpoint("test_light", EndpointDirection::Output, Dpt::BOOL),
            ],
            "function handle(event, input) return { outputs = { test_light = true, nope = false } } end",
        ));
        assert!(matches!(
            run(&mut engine, true, 1).outcome,
            Err(LogicError::InvalidResult { .. })
        ));
        assert!(engine.snapshot().known_inputs.is_empty() == false);
        engine
            .replace_source(
                "function handle(event, input) return { outputs = { test_light = true } } end",
            )
            .unwrap();
        assert_eq!(effects(&run(&mut engine, false, 2)).len(), 1);
    }

    #[test]
    fn invalid_host_events_and_time_reversal_do_not_change_state() {
        let mut engine = Engine::new(config());
        assert!(matches!(
            engine.process_input(
                InputEvent::new(name("test_light"), TypedValue::bool(true)),
                MonotonicMs(1),
            ),
            Err(EventError::EndpointNotInput { .. })
        ));
        assert!(engine.snapshot().known_inputs.is_empty());
        engine
            .observe_input(
                InputObservation::new(name("wall_switch"), TypedValue::bool(true)),
                MonotonicMs(10),
            )
            .unwrap();
        assert!(matches!(
            engine.observe_input(
                InputObservation::new(name("wall_switch"), TypedValue::bool(false)),
                MonotonicMs(9),
            ),
            Err(EventError::TimeWentBackwards { .. })
        ));
        assert!(matches!(
            engine.process_input(trigger(false), MonotonicMs(8)),
            Err(EventError::TimeWentBackwards { .. })
        ));
        let execution = run(&mut engine, false, 11);
        assert_eq!(execution.trigger.previous, Some(TypedValue::bool(true)));
    }
}
