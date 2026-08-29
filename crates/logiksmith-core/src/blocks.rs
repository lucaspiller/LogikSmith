use std::{error::Error, fmt};

use crate::*;

/// Configuration for one independent logic block.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlockConfig {
    pub id: BlockId,
    pub enabled: bool,
    pub endpoints: Vec<Endpoint>,
    pub logic: LogicProgram,
    pub schedules: Vec<BlockSchedule>,
}

impl BlockConfig {
    pub fn new(
        id: BlockId,
        enabled: bool,
        endpoints: Vec<Endpoint>,
        source: impl Into<String>,
    ) -> Self {
        Self {
            id,
            enabled,
            endpoints,
            logic: LogicProgram::new(source),
            schedules: Vec::new(),
        }
    }

    pub fn with_program(
        id: BlockId,
        enabled: bool,
        endpoints: Vec<Endpoint>,
        logic: LogicProgram,
    ) -> Self {
        Self {
            id,
            enabled,
            endpoints,
            logic,
            schedules: Vec::new(),
        }
    }

    /// Builds a block with schedules from a source string.
    pub fn with_schedules(
        id: BlockId,
        enabled: bool,
        endpoints: Vec<Endpoint>,
        source: impl Into<String>,
        schedules: Vec<BlockSchedule>,
    ) -> Self {
        Self {
            id,
            enabled,
            endpoints,
            logic: LogicProgram::new(source),
            schedules,
        }
    }

    pub fn validate(&self) -> Result<(), BlockConfigError> {
        if !self
            .endpoints
            .iter()
            .any(|endpoint| endpoint.direction == EndpointDirection::Input)
        {
            return Err(BlockConfigError::NoInputs);
        }
        if self.schedules.len() > MAX_SCHEDULES_PER_BLOCK {
            return Err(BlockConfigError::TooManySchedules {
                actual: self.schedules.len(),
                maximum: MAX_SCHEDULES_PER_BLOCK,
            });
        }
        for (index, schedule) in self.schedules.iter().enumerate() {
            if self
                .schedules
                .iter()
                .take(index)
                .any(|other| other.name == schedule.name)
            {
                return Err(BlockConfigError::DuplicateSchedule(schedule.name.clone()));
            }
            schedule
                .rule
                .validate()
                .map_err(|error| BlockConfigError::InvalidSchedule {
                    name: schedule.name.clone(),
                    error,
                })?;
        }
        EngineConfig::with_program(self.endpoints.clone(), self.logic.clone())
            .validate()
            .map_err(BlockConfigError::Engine)
    }
}

impl BlockConfig {
    pub fn source(&self) -> &str {
        self.logic.source()
    }

    pub fn logic_program(&self) -> &LogicProgram {
        &self.logic
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BlockConfigError {
    NoInputs,
    TooManySchedules {
        actual: usize,
        maximum: usize,
    },
    DuplicateSchedule(ScheduleName),
    InvalidSchedule {
        name: ScheduleName,
        error: ScheduleError,
    },
    Engine(ConfigError),
}

impl fmt::Display for BlockConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoInputs => formatter.write_str("logic block must define at least one input"),
            Self::TooManySchedules { actual, maximum } => write!(
                formatter,
                "block defines {actual} schedules; maximum is {maximum}"
            ),
            Self::DuplicateSchedule(name) => write!(formatter, "duplicate schedule name {name}"),
            Self::InvalidSchedule { name, error } => {
                write!(formatter, "schedule {name}: {error}")
            }
            Self::Engine(error) => error.fmt(formatter),
        }
    }
}

impl Error for BlockConfigError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Engine(error) => Some(error),
            Self::InvalidSchedule { error, .. } => Some(error),
            _ => None,
        }
    }
}

/// The complete configured set of logic blocks.
#[derive(Clone, Debug, PartialEq)]
pub struct RuntimeConfig {
    pub blocks: Vec<BlockConfig>,
    /// The site wall-clock context is captured from. Defaults to UTC with no
    /// coordinates; hosts assign it (e.g. `config.site = ...`) after
    /// construction.
    pub site: SiteTimeConfig,
}

impl RuntimeConfig {
    pub fn new(blocks: Vec<BlockConfig>) -> Self {
        Self {
            blocks,
            site: SiteTimeConfig {
                timezone: TimeZoneId::utc(),
                coordinates: None,
            },
        }
    }

    pub fn with_site(blocks: Vec<BlockConfig>, site: SiteTimeConfig) -> Self {
        Self { blocks, site }
    }

    pub fn try_new(blocks: Vec<BlockConfig>) -> Result<Self, RuntimeConfigError> {
        let config = Self::new(blocks);
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), RuntimeConfigError> {
        if self.blocks.is_empty() {
            return Err(RuntimeConfigError::Empty);
        }
        if self.blocks.len() > MAX_BLOCKS {
            return Err(RuntimeConfigError::TooMany {
                actual: self.blocks.len(),
                maximum: MAX_BLOCKS,
            });
        }
        for (index, block) in self.blocks.iter().enumerate() {
            if self
                .blocks
                .iter()
                .take(index)
                .any(|other| other.id == block.id)
            {
                return Err(RuntimeConfigError::DuplicateId(block.id.clone()));
            }
            block
                .validate()
                .map_err(|error| RuntimeConfigError::InvalidBlock {
                    block_id: block.id.clone(),
                    error,
                })?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeConfigError {
    Empty,
    TooMany {
        actual: usize,
        maximum: usize,
    },
    DuplicateId(BlockId),
    InvalidBlock {
        block_id: BlockId,
        error: BlockConfigError,
    },
}

impl fmt::Display for RuntimeConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("runtime must contain at least one logic block"),
            Self::TooMany { actual, maximum } => {
                write!(
                    formatter,
                    "runtime contains {actual} blocks; maximum is {maximum}"
                )
            }
            Self::DuplicateId(id) => write!(formatter, "duplicate logic block ID {id}"),
            Self::InvalidBlock { block_id, error } => {
                write!(formatter, "block {block_id}: {error}")
            }
        }
    }
}

impl Error for RuntimeConfigError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidBlock { error, .. } => Some(error),
            _ => None,
        }
    }
}

/// A public view of one block's current semantic state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlockSnapshot {
    pub id: BlockId,
    pub enabled: bool,
    pub logic_revision: LogicRevision,
    pub inputs: Vec<InputSnapshot>,
    pub known_inputs: Vec<(EndpointName, TypedValue)>,
    pub state: TransientState,
    pub pending_timers: Vec<PendingTimer>,
}

/// A public view of every block in declaration order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeSnapshot {
    pub blocks: Vec<BlockSnapshot>,
    pub last_accepted_at: Option<MonotonicMs>,
}

/// An execution tagged with its owning block.
#[derive(Clone, Debug, PartialEq)]
pub struct BlockExecution {
    pub block_id: BlockId,
    pub execution: Execution,
}

impl BlockExecution {
    pub fn block_id(&self) -> &BlockId {
        &self.block_id
    }

    pub fn execution(&self) -> &Execution {
        &self.execution
    }

    pub fn into_execution(self) -> Execution {
        self.execution
    }
}

/// Errors from routing an event to the multi-block runtime.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeEventError {
    UnknownBlock(BlockId),
    Block {
        block_id: BlockId,
        error: EventError,
    },
    TimeWentBackwards {
        block_id: Option<BlockId>,
        previous: MonotonicMs,
        current: MonotonicMs,
    },
}

impl fmt::Display for RuntimeEventError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownBlock(id) => write!(formatter, "unknown logic block {id}"),
            Self::Block { block_id, error } => write!(formatter, "block {block_id}: {error}"),
            Self::TimeWentBackwards {
                block_id,
                previous,
                current,
            } => {
                if let Some(block_id) = block_id {
                    write!(formatter, "block {block_id}: ")?;
                }
                write!(
                    formatter,
                    "event time {current:?} is earlier than the last accepted time {previous:?}"
                )
            }
        }
    }
}

impl Error for RuntimeEventError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Block { error, .. } => Some(error),
            _ => None,
        }
    }
}

/// Errors from block-local simulation routing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeSimulationError {
    UnknownBlock(BlockId),
    Block {
        block_id: BlockId,
        error: SimulationError,
    },
}

impl fmt::Display for RuntimeSimulationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownBlock(id) => write!(formatter, "unknown logic block {id}"),
            Self::Block { block_id, error } => write!(formatter, "block {block_id}: {error}"),
        }
    }
}

impl Error for RuntimeSimulationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Block { error, .. } => Some(error),
            Self::UnknownBlock(_) => None,
        }
    }
}

/// One source/enabled update in an atomic runtime activation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlockActivation {
    pub block_id: BlockId,
    pub source: Option<String>,
    pub enabled: Option<bool>,
}

impl BlockActivation {
    pub fn new(block_id: BlockId, source: Option<String>, enabled: Option<bool>) -> Self {
        Self {
            block_id,
            source,
            enabled,
        }
    }

    pub fn source(block_id: BlockId, source: impl Into<String>) -> Self {
        Self::new(block_id, Some(source.into()), None)
    }

    pub fn enabled(block_id: BlockId, enabled: bool) -> Self {
        Self::new(block_id, None, Some(enabled))
    }
}

/// A validated-before-mutation batch of block updates.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeActivation {
    pub blocks: Vec<BlockActivation>,
}

impl RuntimeActivation {
    pub fn new(blocks: Vec<BlockActivation>) -> Self {
        Self { blocks }
    }

    pub fn single(update: BlockActivation) -> Self {
        Self::new(vec![update])
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlockActivationResult {
    pub block_id: BlockId,
    pub logic_revision: LogicRevision,
    pub enabled: bool,
    pub source_changed: bool,
    pub enabled_changed: bool,
    pub cancelled_timers: Vec<TimerName>,
}

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct ActivationResult {
    pub blocks: Vec<BlockActivationResult>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActivationError {
    UnknownBlock(BlockId),
    DuplicateBlock(BlockId),
    InvalidSource {
        block_id: BlockId,
        error: LogicError,
    },
    EmptyUpdate(BlockId),
}

impl fmt::Display for ActivationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownBlock(id) => write!(formatter, "unknown logic block {id}"),
            Self::DuplicateBlock(id) => write!(formatter, "block {id} appears more than once"),
            Self::InvalidSource { block_id, error } => {
                write!(formatter, "block {block_id} source: {error}")
            }
            Self::EmptyUpdate(id) => write!(formatter, "activation for block {id} changes nothing"),
        }
    }
}

impl Error for ActivationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidSource { error, .. } => Some(error),
            _ => None,
        }
    }
}
