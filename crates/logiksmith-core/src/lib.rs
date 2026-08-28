//! Platform-independent event processing for LogikSmith.
//!
//! The core deals in named, typed endpoints. Hosts provide observations and
//! triggering input events, then execute the logical effects returned by the
//! active Lua program. Transport details such as KNX group addresses stay
//! outside this crate.

use std::{cell::Cell, error::Error, fmt, rc::Rc, str::FromStr};

use mlua::{HookTriggers, Lua, LuaOptions, MultiValue, StdLib, Table, Value as LuaValue, VmState};

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
pub enum Effect {
    SetOutput {
        endpoint: EndpointName,
        value: TypedValue,
    },
}

/// A host-provided monotonic timestamp retained as a small transport-neutral
/// value for desktop diagnostics. The Lua milestone does not schedule work.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct MonotonicMs(pub u64);

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
        }
    }
}

impl Error for EventError {}

/// Failure from either event validation or one contained Lua execution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExecutionError {
    Event(EventError),
    Logic(LogicError),
}

impl fmt::Display for ExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Event(error) => error.fmt(formatter),
            Self::Logic(error) => error.fmt(formatter),
        }
    }
}

impl Error for ExecutionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Event(error) => Some(error),
            Self::Logic(error) => Some(error),
        }
    }
}

impl From<EventError> for ExecutionError {
    fn from(error: EventError) -> Self {
        Self::Event(error)
    }
}

impl From<LogicError> for ExecutionError {
    fn from(error: LogicError) -> Self {
        Self::Logic(error)
    }
}

/// A successful execution, including the revision and trigger used.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Execution {
    pub logic_revision: LogicRevision,
    pub trigger: InputEvent,
    pub effects: Vec<Effect>,
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
}

/// The core event-to-Lua-to-effect engine.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Engine {
    config: EngineConfig,
    known_inputs: Vec<Option<TypedValue>>,
}

impl Engine {
    /// Constructs an engine, panicking if the configuration is invalid.
    /// Prefer [`Self::try_new`] at an external configuration boundary.
    pub fn new(config: EngineConfig) -> Self {
        Self::try_new(config).expect("invalid LogikSmith core configuration")
    }

    pub fn try_new(config: EngineConfig) -> Result<Self, ConfigError> {
        config.validate()?;
        let known_inputs = vec![None; config.endpoints.len()];
        Ok(Self {
            config,
            known_inputs,
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
        }
    }

    /// Returns known values in configured input declaration order.
    pub fn known_input_values(&self) -> Vec<(EndpointName, TypedValue)> {
        self.config
            .endpoints
            .iter()
            .enumerate()
            .filter_map(|(index, endpoint)| {
                (endpoint.direction == EndpointDirection::Input)
                    .then(|| self.known_inputs[index].map(|value| (endpoint.name.clone(), value)))
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
        let program = LogicProgram::try_new(source)?;
        let revision = program.revision;
        self.config.logic = program;
        Ok(revision)
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
        self.config.logic = program;
        Ok(())
    }

    /// Records a value-carrying observation without invoking Lua.
    pub fn observe_input(&mut self, observation: InputObservation) -> Result<(), EventError> {
        let index = self.validate_input(&observation.endpoint, observation.value)?;
        self.known_inputs[index] = Some(observation.value);
        Ok(())
    }

    /// Alias for adapters that model observations as state updates.
    pub fn record_observation(&mut self, observation: InputObservation) -> Result<(), EventError> {
        self.observe_input(observation)
    }

    /// Compatibility entry point for adapters that use the same event shape
    /// for passive response observations and triggering writes.
    pub fn observe(&mut self, event: InputEvent) -> Result<(), EventError> {
        self.observe_input(InputObservation::new(event.endpoint, event.value))
    }

    /// Updates the triggering input before evaluating the active source.
    pub fn process_input(&mut self, event: InputEvent) -> Result<Execution, ExecutionError> {
        let index = self.validate_input(&event.endpoint, event.value)?;
        self.known_inputs[index] = Some(event.value);

        let effects = execute_logic(
            &self.config.endpoints,
            &self.config.logic,
            &self.known_inputs,
            &event,
        )?;
        Ok(Execution {
            logic_revision: self.active_logic_revision(),
            trigger: event,
            effects,
        })
    }

    /// Alias for hosts that call the operation an event rather than an input.
    pub fn process_event(&mut self, event: InputEvent) -> Result<Execution, ExecutionError> {
        self.process_input(event)
    }

    /// Alias retained as the natural event-loop call name.
    pub fn handle_event(&mut self, event: InputEvent) -> Result<Execution, ExecutionError> {
        self.process_input(event)
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
        "assert", "error", "ipairs", "next", "pairs", "select", "tonumber", "tostring", "type",
    ];
    for name in SAFE_BASE {
        let value: LuaValue = globals.get(*name)?;
        environment.set(*name, value)?;
    }
    for name in ["math", "string", "table", "utf8"] {
        let value: LuaValue = globals.get(name)?;
        environment.set(name, value)?;
    }
    Ok(environment)
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
    known_inputs: &[Option<TypedValue>],
    event: &InputEvent,
) -> Result<Vec<Effect>, LogicError> {
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

    let event_table = lua
        .create_table()
        .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
    event_table
        .set("type", "input")
        .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
    event_table
        .set("input", event.endpoint.as_str())
        .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
    event_table
        .set(
            "value",
            typed_value_to_lua(event.value).map_err(|message| LogicError::Runtime {
                message,
                line: None,
            })?,
        )
        .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;

    let input_table = lua
        .create_table()
        .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
    for (index, endpoint) in endpoints.iter().enumerate() {
        if endpoint.direction != EndpointDirection::Input {
            continue;
        }
        if let Some(value) = known_inputs.get(index).copied().flatten() {
            input_table
                .set(
                    endpoint.name.as_str(),
                    typed_value_to_lua(value).map_err(|message| LogicError::Runtime {
                        message,
                        line: None,
                    })?,
                )
                .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
        }
    }

    let returned: MultiValue = handle
        .call((event_table, input_table))
        .map_err(|error| map_lua_error(error, LuaPhase::Runtime))?;
    let mut returned = returned.into_iter();
    let result = returned.next().unwrap_or(LuaValue::Nil);
    if returned.next().is_some() {
        return Err(LogicError::InvalidResult {
            message: "handle must return nil or one result table".to_owned(),
            line: None,
        });
    }
    convert_result(endpoints, result)
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

fn convert_result(endpoints: &[Endpoint], result: LuaValue) -> Result<Vec<Effect>, LogicError> {
    let result_table = match result {
        LuaValue::Nil => return Ok(Vec::new()),
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

    let mut outputs: Option<Table> = None;
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
        if key != "outputs" {
            return Err(LogicError::InvalidResult {
                message: format!("unsupported result field {key:?}; only outputs is allowed"),
                line: None,
            });
        }
        match value {
            LuaValue::Table(table) if outputs.is_none() => outputs = Some(table),
            LuaValue::Table(_) => {
                return Err(LogicError::InvalidResult {
                    message: "result contains duplicate outputs fields".to_owned(),
                    line: None,
                });
            }
            value => {
                return Err(LogicError::InvalidResult {
                    message: format!("outputs must be a table, got {}", value.type_name()),
                    line: None,
                });
            }
        }
    }
    let Some(outputs_table) = outputs else {
        return Ok(Vec::new());
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

    Ok(endpoints
        .iter()
        .enumerate()
        .filter_map(|(index, endpoint)| {
            (endpoint.direction == EndpointDirection::Output)
                .then(|| {
                    values[index].map(|value| Effect::SetOutput {
                        endpoint: endpoint.name.clone(),
                        value,
                    })
                })
                .flatten()
        })
        .collect())
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
        r#"
function handle(event, input)
    if event.input == "wall_switch" and event.value == true then
        return { outputs = { test_light = true, dimmer_output = input.dimmer_level or 0 } }
    end
end
"#
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
            .observe_input(InputObservation::new(
                name("dimmer_level"),
                TypedValue::percent(42).unwrap(),
            ))
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
            .observe_input(InputObservation::new(
                name("dimmer_level"),
                TypedValue::percent(42).unwrap(),
            ))
            .unwrap();
        let execution = engine.process_input(trigger(true)).unwrap();
        assert_eq!(execution.effects.len(), 2);
        assert_eq!(
            execution.effects,
            vec![
                Effect::SetOutput {
                    endpoint: name("test_light"),
                    value: TypedValue::bool(true),
                },
                Effect::SetOutput {
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
    }

    #[test]
    fn nil_empty_and_repeated_results_have_expected_semantics() {
        let mut engine = Engine::new(EngineConfig::new(
            vec![
                endpoint("wall_switch", EndpointDirection::Input, Dpt::BOOL),
                endpoint("test_light", EndpointDirection::Output, Dpt::BOOL),
            ],
            "function handle(event, input) return { outputs = { test_light = event.value } } end",
        ));
        assert_eq!(
            engine.process_input(trigger(true)).unwrap().effects.len(),
            1
        );
        assert_eq!(
            engine.process_input(trigger(true)).unwrap().effects.len(),
            1
        );
        engine
            .replace_source("function handle(event, input) return nil end")
            .unwrap();
        assert!(
            engine
                .process_input(trigger(false))
                .unwrap()
                .effects
                .is_empty()
        );
        engine
            .replace_source("function handle(event, input) return {} end")
            .unwrap();
        assert!(
            engine
                .process_input(trigger(false))
                .unwrap()
                .effects
                .is_empty()
        );
    }

    #[test]
    fn strict_return_conversion_and_all_or_nothing_validation() {
        let mut engine = Engine::new(config());
        engine
            .replace_source(
                "function handle(event, input) return { outputs = { test_light = true, dimmer_output = 12.5 } } end",
            )
            .unwrap();
        assert!(matches!(
            engine.process_input(trigger(true)),
            Err(ExecutionError::Logic(LogicError::InvalidResult { .. }))
        ));
        engine
            .replace_source(
                "function handle(event, input) return { nope = {}, outputs = { test_light = true } } end",
            )
            .unwrap();
        assert!(matches!(
            engine.process_input(trigger(true)),
            Err(ExecutionError::Logic(LogicError::InvalidResult { .. }))
        ));
    }

    #[test]
    fn unsafe_apis_are_unavailable_and_globals_are_fresh() {
        let mut engine = Engine::new(EngineConfig::new(
            vec![
                endpoint("wall_switch", EndpointDirection::Input, Dpt::BOOL),
                endpoint("test_light", EndpointDirection::Output, Dpt::BOOL),
            ],
            "counter = (counter or 0) + 1\nfunction handle(event, input) return { outputs = { test_light = load == nil and counter == 1 } } end",
        ));
        assert_eq!(
            engine.process_input(trigger(true)).unwrap().effects.len(),
            1
        );
        assert_eq!(
            engine.process_input(trigger(true)).unwrap().effects.len(),
            1
        );
        engine
            .replace_source("function handle(event, input) return { outputs = { test_light = io == nil and os == nil and require == nil and debug == nil and coroutine == nil } } end")
            .unwrap();
        assert_eq!(
            engine.process_input(trigger(true)).unwrap().effects.len(),
            1
        );
    }

    #[test]
    fn instruction_limit_fails_and_next_event_recovers() {
        let mut engine = Engine::new(EngineConfig::new(
            vec![
                endpoint("wall_switch", EndpointDirection::Input, Dpt::BOOL),
                endpoint("test_light", EndpointDirection::Output, Dpt::BOOL),
            ],
            "function handle(event, input) while true do end end",
        ));
        assert!(matches!(
            engine.process_input(trigger(true)),
            Err(ExecutionError::Logic(LogicError::InstructionLimit { .. }))
        ));
        engine
            .replace_source(
                "function handle(event, input) return { outputs = { test_light = true } } end",
            )
            .unwrap();
        assert_eq!(
            engine.process_input(trigger(true)).unwrap().effects.len(),
            1
        );
    }

    #[test]
    fn memory_limit_fails_and_next_event_recovers() {
        let mut engine = Engine::new(EngineConfig::new(
            vec![
                endpoint("wall_switch", EndpointDirection::Input, Dpt::BOOL),
                endpoint("test_light", EndpointDirection::Output, Dpt::BOOL),
            ],
            "function handle(event, input) local value = string.rep('x', 2 * 1024 * 1024) return { outputs = { test_light = true } } end",
        ));
        assert!(matches!(
            engine.process_input(trigger(true)),
            Err(ExecutionError::Logic(LogicError::MemoryLimit { .. }))
        ));
        engine
            .replace_source(
                "function handle(event, input) return { outputs = { test_light = true } } end",
            )
            .unwrap();
        assert_eq!(
            engine.process_input(trigger(true)).unwrap().effects.len(),
            1
        );
    }

    #[test]
    fn event_validation_rejects_outputs_and_wrong_dpts() {
        let mut engine = Engine::new(config());
        assert!(matches!(
            engine.process_input(InputEvent::new(name("test_light"), TypedValue::bool(true))),
            Err(ExecutionError::Event(EventError::EndpointNotInput { .. }))
        ));
        assert!(matches!(
            engine.process_input(InputEvent::new(
                name("wall_switch"),
                TypedValue::percent(42).unwrap()
            )),
            Err(ExecutionError::Event(EventError::DptMismatch { .. }))
        ));
    }
}
