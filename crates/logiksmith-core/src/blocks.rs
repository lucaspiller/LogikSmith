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
    /// Bindings to globally declared internal signals. Input endpoints
    /// consume a signal; output endpoints publish one.
    pub signal_bindings: Vec<SignalBinding>,
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
            signal_bindings: Vec::new(),
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
            signal_bindings: Vec::new(),
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
            signal_bindings: Vec::new(),
        }
    }

    pub fn validate(&self) -> Result<(), BlockConfigError> {
        self.validate_with_limits(&RuntimeLimits::desktop())
    }

    pub fn validate_with_limits(&self, limits: &RuntimeLimits) -> Result<(), BlockConfigError> {
        if self.endpoints.len() > limits.max_endpoints_per_block {
            return Err(BlockConfigError::TooManyEndpoints {
                actual: self.endpoints.len(),
                maximum: limits.max_endpoints_per_block,
            });
        }
        if self.logic.source.len() > limits.max_logic_source_bytes_per_block {
            return Err(BlockConfigError::LogicSourceTooLarge {
                actual: self.logic.source.len(),
                maximum: limits.max_logic_source_bytes_per_block,
            });
        }
        if self.schedules.len() > limits.max_schedules_per_block {
            return Err(BlockConfigError::TooManySchedules {
                actual: self.schedules.len(),
                maximum: limits.max_schedules_per_block,
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
        for (index, binding) in self.signal_bindings.iter().enumerate() {
            if self
                .signal_bindings
                .iter()
                .take(index)
                .any(|other| other.endpoint == binding.endpoint)
            {
                return Err(BlockConfigError::DuplicateSignalBinding {
                    endpoint: binding.endpoint.clone(),
                });
            }
            let Some(_endpoint) = self
                .endpoints
                .iter()
                .find(|endpoint| endpoint.name == binding.endpoint)
            else {
                return Err(BlockConfigError::UnknownSignalBindingEndpoint {
                    endpoint: binding.endpoint.clone(),
                });
            };
        }
        EngineConfig::with_program(self.endpoints.clone(), self.logic.clone())
            .validate_with_limits(limits)
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
    TooManyEndpoints {
        actual: usize,
        maximum: usize,
    },
    LogicSourceTooLarge {
        actual: usize,
        maximum: usize,
    },
    TooManySchedules {
        actual: usize,
        maximum: usize,
    },
    DuplicateSchedule(ScheduleName),
    InvalidSchedule {
        name: ScheduleName,
        error: ScheduleError,
    },
    DuplicateSignalBinding {
        endpoint: EndpointName,
    },
    UnknownSignalBindingEndpoint {
        endpoint: EndpointName,
    },
    Engine(ConfigError),
}

impl fmt::Display for BlockConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyEndpoints { actual, maximum } => write!(
                formatter,
                "block defines {actual} endpoints; maximum is {maximum}"
            ),
            Self::LogicSourceTooLarge { actual, maximum } => write!(
                formatter,
                "logic source is {actual} bytes; maximum is {maximum}"
            ),
            Self::TooManySchedules { actual, maximum } => write!(
                formatter,
                "block defines {actual} schedules; maximum is {maximum}"
            ),
            Self::DuplicateSchedule(name) => write!(formatter, "duplicate schedule name {name}"),
            Self::InvalidSchedule { name, error } => {
                write!(formatter, "schedule {name}: {error}")
            }
            Self::DuplicateSignalBinding { endpoint } => {
                write!(
                    formatter,
                    "endpoint {endpoint} has more than one signal binding"
                )
            }
            Self::UnknownSignalBindingEndpoint { endpoint } => {
                write!(
                    formatter,
                    "signal binding references unknown endpoint {endpoint}"
                )
            }
            Self::Engine(error) => error.fmt(formatter),
        }
    }
}

/// A declared internal signal and its exact DPT.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignalConfig {
    pub name: SignalName,
    pub dpt: Dpt,
}

impl SignalConfig {
    pub fn new(name: SignalName, dpt: Dpt) -> Self {
        Self { name, dpt }
    }
}

/// Binds one block endpoint to a global signal.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignalBinding {
    pub endpoint: EndpointName,
    pub signal: SignalName,
}

impl SignalBinding {
    pub fn new(endpoint: EndpointName, signal: SignalName) -> Self {
        Self { endpoint, signal }
    }
}

pub const MAX_SIGNALS: usize = 256;
pub const MAX_SIGNAL_BINDINGS: usize = 256;

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
    pub signals: Vec<SignalConfig>,
    /// The site wall-clock context is captured from. Defaults to UTC with no
    /// coordinates; hosts assign it (e.g. `config.site = ...`) after
    /// construction.
    pub site: SiteTimeConfig,
}

impl RuntimeConfig {
    pub fn new(blocks: Vec<BlockConfig>) -> Self {
        Self {
            blocks,
            signals: Vec::new(),
            site: SiteTimeConfig {
                timezone: TimeZoneId::utc(),
                coordinates: None,
            },
        }
    }

    pub fn with_site(blocks: Vec<BlockConfig>, site: SiteTimeConfig) -> Self {
        Self {
            blocks,
            signals: Vec::new(),
            site,
        }
    }

    pub fn with_signals(blocks: Vec<BlockConfig>, signals: Vec<SignalConfig>) -> Self {
        Self {
            blocks,
            signals,
            site: SiteTimeConfig {
                timezone: TimeZoneId::utc(),
                coordinates: None,
            },
        }
    }

    pub fn with_signals_and_site(
        blocks: Vec<BlockConfig>,
        signals: Vec<SignalConfig>,
        site: SiteTimeConfig,
    ) -> Self {
        Self {
            blocks,
            signals,
            site,
        }
    }

    pub fn try_new(blocks: Vec<BlockConfig>) -> Result<Self, RuntimeConfigError> {
        let config = Self::new(blocks);
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), RuntimeConfigError> {
        self.validate_with_limits(&RuntimeLimits::desktop())
    }

    pub fn validate_with_limits(&self, limits: &RuntimeLimits) -> Result<(), RuntimeConfigError> {
        if self.blocks.is_empty() {
            return Err(RuntimeConfigError::Empty);
        }
        if self.blocks.len() > limits.max_logic_blocks {
            return Err(RuntimeConfigError::TooMany {
                actual: self.blocks.len(),
                maximum: limits.max_logic_blocks,
            });
        }
        if self.signals.len() > limits.max_signals {
            return Err(RuntimeConfigError::TooManySignals {
                actual: self.signals.len(),
                maximum: limits.max_signals,
            });
        }
        let mut signal_names = std::collections::BTreeSet::new();
        for signal in &self.signals {
            if !signal_names.insert(signal.name.clone()) {
                return Err(RuntimeConfigError::DuplicateSignal(signal.name.clone()));
            }
            if !signal.dpt.is_supported() {
                return Err(RuntimeConfigError::UnsupportedSignalDpt {
                    signal: signal.name.clone(),
                    dpt: signal.dpt,
                });
            }
        }
        let binding_count: usize = self
            .blocks
            .iter()
            .map(|block| block.signal_bindings.len())
            .sum();
        if binding_count > limits.max_signal_bindings {
            return Err(RuntimeConfigError::TooManySignalBindings {
                actual: binding_count,
                maximum: limits.max_signal_bindings,
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
            block.validate_with_limits(limits).map_err(|error| {
                RuntimeConfigError::InvalidBlock {
                    block_id: block.id.clone(),
                    error,
                }
            })?;
        }
        let source_bytes: usize = self.blocks.iter().fold(0usize, |total, block| {
            total.saturating_add(block.logic.source.len())
        });
        if source_bytes > limits.max_logic_source_bytes_total {
            return Err(RuntimeConfigError::TooMuchLogicSource {
                actual: source_bytes,
                maximum: limits.max_logic_source_bytes_total,
            });
        }
        let mut producers: std::collections::BTreeMap<SignalName, SignalEndpointId> =
            std::collections::BTreeMap::new();
        let mut edges: std::collections::BTreeMap<BlockId, Vec<BlockId>> =
            std::collections::BTreeMap::new();
        for block in &self.blocks {
            for binding in &block.signal_bindings {
                let signal = self
                    .signals
                    .iter()
                    .find(|signal| signal.name == binding.signal)
                    .ok_or_else(|| RuntimeConfigError::UnknownSignal {
                        block_id: block.id.clone(),
                        endpoint: binding.endpoint.clone(),
                        signal: binding.signal.clone(),
                    })?;
                let endpoint = block
                    .endpoints
                    .iter()
                    .find(|endpoint| endpoint.name == binding.endpoint)
                    .expect("BlockConfig validates signal endpoint references");
                if endpoint.dpt != signal.dpt {
                    return Err(RuntimeConfigError::SignalDptMismatch {
                        block_id: block.id.clone(),
                        endpoint: endpoint.name.clone(),
                        signal: signal.name.clone(),
                        expected: endpoint.dpt,
                        actual: signal.dpt,
                    });
                }
                let identity = SignalEndpointId::new(block.id.clone(), endpoint.name.clone());
                if endpoint.direction == EndpointDirection::Output {
                    if let Some(previous) = producers.insert(signal.name.clone(), identity.clone())
                    {
                        return Err(RuntimeConfigError::DuplicateSignalProducer {
                            signal: signal.name.clone(),
                            previous,
                            duplicate: identity,
                        });
                    }
                }
            }
        }
        // A producer may be declared later than its consumer, so resolve all
        // input edges in a second pass after collecting producers.
        edges.clear();
        for block in &self.blocks {
            for binding in &block.signal_bindings {
                let endpoint = block
                    .endpoints
                    .iter()
                    .find(|endpoint| endpoint.name == binding.endpoint)
                    .unwrap();
                if endpoint.direction == EndpointDirection::Input
                    && let Some(producer) = producers.get(&binding.signal)
                {
                    edges
                        .entry(producer.block_id.clone())
                        .or_default()
                        .push(block.id.clone());
                }
            }
        }
        detect_signal_cycle(&self.blocks, &edges)?;
        let cascade_executions = self
            .blocks
            .iter()
            .map(|block| cascade_size(&block.id, &edges, &mut std::collections::BTreeMap::new()))
            .max()
            .unwrap_or(0);
        if cascade_executions > limits.max_cascade_executions {
            return Err(RuntimeConfigError::CascadeLimit {
                actual: cascade_executions,
                maximum: limits.max_cascade_executions,
            });
        }
        Ok(())
    }
}

fn cascade_size(
    id: &BlockId,
    edges: &std::collections::BTreeMap<BlockId, Vec<BlockId>>,
    memo: &mut std::collections::BTreeMap<BlockId, usize>,
) -> usize {
    if let Some(size) = memo.get(id) {
        return *size;
    }
    let size = 1usize.saturating_add(
        edges
            .get(id)
            .into_iter()
            .flatten()
            .map(|child| cascade_size(child, edges, memo))
            .sum(),
    );
    memo.insert(id.clone(), size);
    size
}

fn detect_signal_cycle(
    blocks: &[BlockConfig],
    edges: &std::collections::BTreeMap<BlockId, Vec<BlockId>>,
) -> Result<(), RuntimeConfigError> {
    fn visit(
        id: &BlockId,
        edges: &std::collections::BTreeMap<BlockId, Vec<BlockId>>,
        visiting: &mut std::collections::BTreeSet<BlockId>,
        visited: &mut std::collections::BTreeSet<BlockId>,
        path: &mut Vec<BlockId>,
    ) -> Result<(), Vec<BlockId>> {
        if visiting.contains(id) {
            let start = path.iter().position(|item| item == id).unwrap_or(0);
            let mut cycle = path[start..].to_vec();
            cycle.push(id.clone());
            return Err(cycle);
        }
        if !visited.insert(id.clone()) {
            return Ok(());
        }
        visiting.insert(id.clone());
        path.push(id.clone());
        if let Some(children) = edges.get(id) {
            for child in children {
                // A node can be reached from two branches; `visited` is only
                // committed after its DFS completes so back edges remain
                // detectable.
                if let Err(cycle) = visit(child, edges, visiting, visited, path) {
                    return Err(cycle);
                }
            }
        }
        path.pop();
        visiting.remove(id);
        Ok(())
    }

    let mut visiting = std::collections::BTreeSet::new();
    let mut visited = std::collections::BTreeSet::new();
    let mut path = Vec::new();
    for block in blocks {
        if !visited.contains(&block.id)
            && let Err(path) = visit(&block.id, edges, &mut visiting, &mut visited, &mut path)
        {
            return Err(RuntimeConfigError::SignalCycle { path });
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeConfigError {
    Empty,
    TooMany {
        actual: usize,
        maximum: usize,
    },
    DuplicateId(BlockId),
    TooManySignals {
        actual: usize,
        maximum: usize,
    },
    DuplicateSignal(SignalName),
    UnsupportedSignalDpt {
        signal: SignalName,
        dpt: Dpt,
    },
    TooManySignalBindings {
        actual: usize,
        maximum: usize,
    },
    TooMuchLogicSource {
        actual: usize,
        maximum: usize,
    },
    CascadeLimit {
        actual: usize,
        maximum: usize,
    },
    UnknownSignal {
        block_id: BlockId,
        endpoint: EndpointName,
        signal: SignalName,
    },
    SignalDptMismatch {
        block_id: BlockId,
        endpoint: EndpointName,
        signal: SignalName,
        expected: Dpt,
        actual: Dpt,
    },
    DuplicateSignalProducer {
        signal: SignalName,
        previous: SignalEndpointId,
        duplicate: SignalEndpointId,
    },
    SignalCycle {
        path: Vec<BlockId>,
    },
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
            Self::TooManySignals { actual, maximum } => write!(
                formatter,
                "runtime contains {actual} signals; maximum is {maximum}"
            ),
            Self::DuplicateSignal(name) => write!(formatter, "duplicate signal {name}"),
            Self::UnsupportedSignalDpt { signal, dpt } => {
                write!(formatter, "signal {signal} uses unsupported DPT {dpt}")
            }
            Self::TooManySignalBindings { actual, maximum } => write!(
                formatter,
                "runtime contains {actual} signal bindings; maximum is {maximum}"
            ),
            Self::TooMuchLogicSource { actual, maximum } => write!(
                formatter,
                "runtime logic sources use {actual} bytes; maximum is {maximum}"
            ),
            Self::CascadeLimit { actual, maximum } => write!(
                formatter,
                "maximum signal cascade contains {actual} executions; maximum is {maximum}"
            ),
            Self::UnknownSignal {
                block_id,
                endpoint,
                signal,
            } => write!(
                formatter,
                "block {block_id} endpoint {endpoint} references unknown signal {signal}"
            ),
            Self::SignalDptMismatch {
                block_id,
                endpoint,
                signal,
                expected,
                actual,
            } => write!(
                formatter,
                "block {block_id} endpoint {endpoint} DPT {expected} does not match signal {signal} DPT {actual}"
            ),
            Self::DuplicateSignalProducer {
                signal,
                previous,
                duplicate,
            } => write!(
                formatter,
                "signal {signal} has producers {previous} and {duplicate}"
            ),
            Self::SignalCycle { path } => write!(
                formatter,
                "signal dependency cycle: {}",
                path.iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(" -> ")
            ),
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

include!("block_health.rs");

/// A public view of every block in declaration order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeSnapshot {
    pub blocks: Vec<BlockSnapshot>,
    pub signals: Vec<SignalSnapshot>,
    pub last_accepted_at: Option<MonotonicMs>,
}

/// Monotonic identity assigned by a live runtime to every executed block.
pub type ExecutionId = u64;

/// Current status of a declared signal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SignalStatus {
    Unknown,
    Valid,
    ProducerDisabled,
}

impl SignalStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Valid => "valid",
            Self::ProducerDisabled => "producer_disabled",
        }
    }
}

impl fmt::Display for SignalStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One endpoint identity in a signal's producer/consumer graph.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SignalEndpointId {
    pub block_id: BlockId,
    pub endpoint: EndpointName,
}

impl SignalEndpointId {
    pub fn new(block_id: BlockId, endpoint: EndpointName) -> Self {
        Self { block_id, endpoint }
    }
}

impl fmt::Display for SignalEndpointId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}", self.block_id, self.endpoint)
    }
}

/// A committed or proposed value produced for one signal.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignalEffect {
    pub signal: SignalName,
    pub value: TypedValue,
    pub changed: bool,
    pub producer: SignalEndpointId,
    pub producing_execution: Option<ExecutionId>,
    pub consumers: Vec<SignalEndpointId>,
}

/// A public view of one signal's in-memory state and graph identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignalSnapshot {
    pub name: SignalName,
    pub dpt: Dpt,
    pub value: Option<TypedValue>,
    pub status: SignalStatus,
    pub observed_at: Option<MonotonicMs>,
    pub changed_at: Option<MonotonicMs>,
    pub producer: Option<SignalEndpointId>,
    pub producing_execution: Option<ExecutionId>,
    pub consumers: Vec<SignalEndpointId>,
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
    CascadeLimit {
        actual: usize,
        maximum: usize,
    },
    ResourceLimit {
        resource: &'static str,
        actual: usize,
        maximum: usize,
    },
    CascadeTimeLimit {
        elapsed_ms: u64,
        maximum_ms: u64,
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
            Self::CascadeLimit { actual, maximum } => write!(
                formatter,
                "signal cascade contains {actual} executions; maximum is {maximum}"
            ),
            Self::ResourceLimit {
                resource,
                actual,
                maximum,
            } => write!(formatter, "{resource} uses {actual}; maximum is {maximum}"),
            Self::CascadeTimeLimit {
                elapsed_ms,
                maximum_ms,
            } => write!(
                formatter,
                "signal cascade took {elapsed_ms} ms; maximum is {maximum_ms} ms"
            ),
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
