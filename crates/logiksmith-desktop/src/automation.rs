use crate::wire_revision;
use crate::*;
use logiksmith_core::{
    BlockActivation as CoreBlockActivation, BlockId, BlockSchedule as CoreBlockSchedule, Dpt,
    EndpointName, EngineConfig, RuntimeConfig as CoreRuntimeConfig, SignalName,
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fmt, io, path::PathBuf, str::FromStr};
use thiserror::Error;
use tokio::sync::oneshot;
/// A source replacement requested by the management API. The session owns the
/// engine, so activation is acknowledged on that same task between events.
pub struct ActivationRequest {
    pub updates: Vec<CoreBlockActivation>,
    pub document_revision: u64,
    /// The saved document whose source/enabled fields are represented by the
    /// activation. The session copies this into its active projection only
    /// after the core accepts the atomic activation.
    pub document: AutomationDocument,
    pub reply: oneshot::Sender<Result<CoreActivationResult, String>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoreActivationResult {
    pub document_revision: u64,
    pub result: logiksmith_core::ActivationResult,
}

/// Browser payload for one immutable input simulation. Values carry their
/// semantic kind; the configured endpoint supplies the DPT.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SimulationPayload {
    #[serde(alias = "blockId")]
    pub block_id: String,
    #[serde(alias = "expectedLogicRevision")]
    #[serde(deserialize_with = "wire_revision::deserialize")]
    pub expected_logic_revision: u64,
    #[serde(default, alias = "expectedStructuralRevision")]
    #[serde(deserialize_with = "wire_revision::deserialize_option")]
    pub expected_structural_revision: Option<u64>,
    /// Schedule-preview-only cursor. It is intentionally distinct from the
    /// selected occurrence so preview requests need no Lua scenario fields.
    #[serde(default, alias = "afterUtcMs")]
    pub preview_after_utc_ms: Option<i64>,
    #[serde(default)]
    pub preview_count: Option<usize>,
    pub trigger: SimulationTriggerPayload,
    pub inputs: Vec<SimulationInputPayload>,
    #[serde(default)]
    pub state: Option<std::collections::BTreeMap<String, StateValuePayload>>,
    #[serde(default, alias = "pendingTimers")]
    pub pending_timers: Option<Vec<PendingTimerPayload>>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SimulationTriggerPayload {
    #[serde(rename = "type", default)]
    pub trigger_type: Option<String>,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub value: Option<ValueMessage>,
    #[serde(default)]
    pub previous: Option<ValueMessage>,
    #[serde(default)]
    #[serde(alias = "timer")]
    pub name: Option<String>,
    #[serde(default)]
    #[serde(alias = "firedAtMs")]
    pub fired_at_ms: Option<u64>,
    #[serde(default)]
    pub schedule: Option<String>,
    #[serde(default, alias = "occurrenceAtMs")]
    pub occurrence_at_ms: Option<i64>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateValuePayload {
    pub kind: String,
    pub value: serde_json::Value,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingTimerPayload {
    pub name: String,
    #[serde(alias = "scheduledAtMs")]
    pub scheduled_at_ms: u64,
    #[serde(alias = "dueAtMs")]
    pub due_at_ms: u64,
    #[serde(alias = "logicRevision")]
    #[serde(deserialize_with = "wire_revision::deserialize")]
    pub logic_revision: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SimulationInputPayload {
    pub endpoint: String,
    pub value: Option<ValueMessage>,
    pub valid: bool,
    #[serde(alias = "ageMs")]
    pub age_ms: Option<u64>,
}

/// A request sent to the runtime owner. The HTTP handler never evaluates Lua
/// itself, so bridge events, activation, and simulation remain serialized by
/// the session actor.
pub struct SimulationRequest {
    pub payload: SimulationPayload,
    pub reply: oneshot::Sender<SimulationOutcome>,
}

pub enum SimulationOutcome {
    NotFound,
    /// A dedicated schedule request named a block-local schedule that does
    /// not exist. This is kept separate from `NotFound` so the HTTP layer can
    /// preserve the schedule endpoint's public 404 contract.
    ScheduleNotFound,
    Conflict {
        current_revision: u64,
    },
    /// Dedicated schedule simulation requests expose both revision tokens so
    /// callers can refresh the right part of their request after a conflict.
    ScheduleConflict {
        current_revision: u64,
        current_structural_revision: u64,
    },
    Invalid(Vec<FieldError>),
    Complete(diagnostics::SimulationResponse),
    /// Read-only occurrence preview for a schedule trigger simulation.
    Previews(diagnostics::SchedulePreviewResponse),
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GroupAddress {
    pub(crate) main: u8,
    pub(crate) middle: u8,
    pub(crate) subgroup: u8,
}

impl GroupAddress {
    pub fn parse(value: &str) -> Result<Self, GroupAddressError> {
        value.parse()
    }
}

impl FromStr for GroupAddress {
    type Err = GroupAddressError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let parts: Vec<_> = value.split('/').collect();
        if parts.len() != 3
            || parts
                .iter()
                .any(|part| part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()))
        {
            return Err(GroupAddressError::InvalidFormat);
        }
        let main = parts[0]
            .parse::<u16>()
            .map_err(|_| GroupAddressError::InvalidFormat)?;
        let middle = parts[1]
            .parse::<u16>()
            .map_err(|_| GroupAddressError::InvalidFormat)?;
        let subgroup = parts[2]
            .parse::<u16>()
            .map_err(|_| GroupAddressError::InvalidFormat)?;
        if main > 31 || middle > 7 || subgroup > 255 {
            return Err(GroupAddressError::OutOfRange);
        }
        if main == 0 && middle == 0 && subgroup == 0 {
            return Err(GroupAddressError::BroadcastReserved);
        }
        if parts
            .iter()
            .any(|part| part.len() > 1 && part.starts_with('0'))
        {
            return Err(GroupAddressError::NonCanonical);
        }
        Ok(Self {
            main: main as u8,
            middle: middle as u8,
            subgroup: subgroup as u8,
        })
    }
}

impl fmt::Display for GroupAddress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}/{}/{}", self.main, self.middle, self.subgroup)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GroupAddressError {
    InvalidFormat,
    OutOfRange,
    BroadcastReserved,
    NonCanonical,
}

impl fmt::Display for GroupAddressError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidFormat => "group address must be main/middle/subgroup",
            Self::OutOfRange => "group address component is out of range",
            Self::BroadcastReserved => "group address 0/0/0 is reserved for broadcast",
            Self::NonCanonical => "group address must use canonical main/middle/subgroup form",
        })
    }
}

impl std::error::Error for GroupAddressError {}

// ---------------------------------------------------------------------------
// Host and automation configuration

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub config_path: PathBuf,
    pub automation_path: PathBuf,
    pub automation: AutomationRuntime,
    pub automation_revision: u64,
    pub connection: ConnectionConfig,
    pub bridge: BridgeConfig,
    pub logging: LoggingConfig,
    pub web: WebConfig,
}

#[derive(Debug, Clone)]
pub struct AutomationRuntime {
    pub document: AutomationDocument,
    pub signals: Vec<SignalRuntime>,
    pub core_config: CoreRuntimeConfig,
    pub blocks: Vec<BlockRuntime>,
    pub address_to_inputs: HashMap<GroupAddress, Vec<BlockInputBinding>>,
    pub output_to_address: HashMap<(BlockId, EndpointName), GroupAddress>,
    pub signal_to_inputs: HashMap<SignalName, Vec<BlockSignalInputBinding>>,
    pub output_to_signal: HashMap<(BlockId, EndpointName), SignalName>,
    pub address_dpts: HashMap<GroupAddress, Dpt>,
    pub structural_revision: u64,
    pub document_revision: u64,
}

#[derive(Debug, Clone)]
pub struct BlockRuntime {
    pub id: BlockId,
    pub revision: u64,
    pub enabled: bool,
    pub engine_config: EngineConfig,
    pub endpoint_to_address: HashMap<EndpointName, GroupAddress>,
    pub endpoint_to_signal: HashMap<EndpointName, SignalName>,
    pub endpoint_dpts: HashMap<EndpointName, Dpt>,
    /// Validated schedule definitions owned by this block. The desktop keeps
    /// them beside the KNX maps for diagnostics and simulation routing; the
    /// core scheduler owns the authoritative copy.
    pub schedules: Vec<CoreBlockSchedule>,
}

#[derive(Debug, Clone)]
pub struct BlockInputBinding {
    pub block_id: BlockId,
    pub endpoint: EndpointName,
    pub dpt: Dpt,
    pub address: GroupAddress,
}

#[derive(Debug, Clone)]
pub struct SignalRuntime {
    pub name: SignalName,
    pub dpt: Dpt,
}

#[derive(Debug, Clone)]
pub struct BlockSignalInputBinding {
    pub block_id: BlockId,
    pub endpoint: EndpointName,
    pub dpt: Dpt,
    pub signal: SignalName,
}

impl AutomationRuntime {
    pub fn block(&self, id: &BlockId) -> Option<&BlockRuntime> {
        self.blocks.iter().find(|block| block.id == *id)
    }

    pub fn block_ids(&self) -> impl Iterator<Item = &BlockId> {
        self.blocks.iter().map(|block| &block.id)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AutomationDocument {
    #[serde(default)]
    pub signals: Vec<AutomationSignal>,
    pub blocks: Vec<AutomationBlock>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AutomationSignal {
    pub name: String,
    pub dpt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AutomationBlock {
    pub id: String,
    #[serde(default = "default_block_revision")]
    pub revision: u64,
    pub enabled: bool,
    #[serde(default)]
    pub inputs: Vec<AutomationEndpoint>,
    #[serde(default)]
    pub outputs: Vec<AutomationEndpoint>,
    #[serde(default)]
    pub knx_bindings: Vec<KnxBinding>,
    #[serde(default)]
    pub signal_bindings: Vec<SignalBinding>,
    pub source: String,
    #[serde(default)]
    pub schedules: Vec<AutomationSchedule>,
}

fn default_block_revision() -> u64 {
    1
}

/// One configured schedule inside a block document. Kind-specific fields are
/// closed per `kind`; the flatten map captures unknown fields so validation
/// can reject them with precise paths instead of silently dropping them.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AutomationSchedule {
    pub name: String,
    pub enabled: bool,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub every: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weekdays: Option<Vec<String>>,
    #[serde(flatten)]
    pub extra: std::collections::BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredAutomation {
    #[serde(default, skip_serializing)]
    pub(crate) revision: u16,
    #[serde(flatten)]
    pub(crate) document: AutomationDocument,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AutomationEndpoint {
    pub name: String,
    pub dpt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct KnxBinding {
    pub endpoint: String,
    pub group_address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SignalBinding {
    pub endpoint: String,
    pub signal: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AutomationEnvelope {
    pub document: AutomationDocument,
    #[serde(skip_serializing)]
    pub revision: u64,
    #[serde(serialize_with = "wire_revision::serialize")]
    pub active_structural_revision: u64,
    #[serde(serialize_with = "wire_revision::serialize")]
    pub saved_structural_revision: u64,
    #[serde(serialize_with = "wire_revision::serialize")]
    pub active_logic_revision: u64,
    #[serde(serialize_with = "wire_revision::serialize")]
    pub saved_logic_revision: u64,
    pub restart_required: bool,
    pub blocks: Vec<AutomationBlockStatus>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AutomationBlockStatus {
    pub id: String,
    pub active_enabled: bool,
    pub saved_enabled: bool,
    #[serde(serialize_with = "wire_revision::serialize")]
    pub active_revision: u64,
    #[serde(serialize_with = "wire_revision::serialize")]
    pub saved_revision: u64,
    #[serde(serialize_with = "wire_revision::serialize")]
    pub active_logic_revision: u64,
    #[serde(serialize_with = "wire_revision::serialize")]
    pub saved_logic_revision: u64,
}

pub fn block_revision(document: &AutomationDocument, id: &str) -> u64 {
    document
        .blocks
        .iter()
        .find(|block| block.id == id)
        .map(|block| block.revision.max(1))
        .unwrap_or(1)
}

#[derive(Debug, Clone, Serialize)]
pub struct FieldError {
    pub path: String,
    pub message: String,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("cannot read configuration {path}: {source}")]
    Read { path: PathBuf, source: io::Error },
    #[error("invalid TOML configuration: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("invalid configuration field {field}: {message}")]
    Field { field: String, message: String },
    #[error("cannot read automation file {path}: {source}")]
    AutomationRead { path: PathBuf, source: io::Error },
    #[error("invalid automation TOML: {0}")]
    AutomationToml(toml::de::Error),
    #[error("invalid automation configuration")]
    AutomationInvalid(Vec<FieldError>),
}

#[derive(Debug, Error)]
pub enum AutomationFileError {
    #[error("cannot read automation file {path}: {source}")]
    Read { path: PathBuf, source: io::Error },
    #[error("invalid automation TOML: {0}")]
    Toml(toml::de::Error),
    #[error("invalid automation configuration")]
    Invalid(Vec<FieldError>),
}
