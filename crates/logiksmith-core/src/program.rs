use std::{error::Error, fmt};

use crate::lua::validate_logic_source;
use crate::{
    Dpt, EndpointDirection, EndpointName, LogicRevision, MonotonicMs, PendingTimer, StateError,
    TimerName, TransientState, TypedValue, ValueError,
};

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

pub(crate) fn revision_for(source: &str) -> LogicRevision {
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
    /// The supplied draft source failed the same restricted load checks used
    /// by live activation. It is kept separate from scenario validation so a
    /// host can project a source diagnostic without changing live state.
    InvalidSource(LogicError),
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
            Self::InvalidSource(error) => write!(formatter, "invalid simulation source: {error}"),
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
