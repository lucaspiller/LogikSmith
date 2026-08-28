//! Tokio desktop host for the platform-independent LogikSmith engine.

pub mod diagnostics;
pub mod web;

use diagnostics::{ConnectionState, DiagnosticStore, TelegramRecord};
use logiksmith_core::{
    Dpt, Effect, Endpoint, EndpointDirection, EndpointName, Engine, EngineConfig, InputEvent,
    InputObservation, SimulationError, SimulationInput, SimulationScenario, SimulationTrigger,
    TypedValue, Value,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    fmt, fs, io,
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    str::FromStr,
    time::{Duration, Instant},
};
use thiserror::Error;
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    signal,
    sync::{mpsc, oneshot},
    time,
};
use tracing_subscriber::{
    EnvFilter, Layer, filter::LevelFilter, layer::SubscriberExt, util::SubscriberInitExt,
};
use web::WebError;

pub const PROTOCOL_VERSION: u64 = 1;

/// A source replacement requested by the management API. The session owns the
/// engine, so activation is acknowledged on that same task between events.
pub struct ActivationRequest {
    pub source: String,
    pub logic_hash: u64,
    pub document_revision: u64,
    pub reply: oneshot::Sender<Result<(), String>>,
}

/// Browser payload for one immutable input simulation. Values carry their
/// semantic kind; the configured endpoint supplies the DPT.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SimulationPayload {
    pub expected_logic_revision: u64,
    pub trigger: SimulationTriggerPayload,
    pub inputs: Vec<SimulationInputPayload>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SimulationTriggerPayload {
    pub endpoint: String,
    pub value: ValueMessage,
    #[serde(default)]
    pub previous: Option<ValueMessage>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SimulationInputPayload {
    pub endpoint: String,
    pub value: Option<ValueMessage>,
    pub valid: bool,
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
    Conflict { current_revision: u64 },
    Invalid(Vec<FieldError>),
    Complete(diagnostics::SimulationResponse),
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GroupAddress {
    main: u8,
    middle: u8,
    subgroup: u8,
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
    pub engine_config: EngineConfig,
    pub address_to_endpoint: HashMap<GroupAddress, BindingRuntime>,
    pub endpoint_to_address: HashMap<EndpointName, GroupAddress>,
    pub endpoint_dpts: HashMap<EndpointName, Dpt>,
    pub structural_revision: u64,
    pub document_revision: u64,
}

#[derive(Debug, Clone)]
pub struct BindingRuntime {
    pub endpoint: EndpointName,
    pub direction: EndpointDirection,
    pub dpt: Dpt,
    pub address: GroupAddress,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AutomationDocument {
    pub inputs: Vec<AutomationEndpoint>,
    pub outputs: Vec<AutomationEndpoint>,
    pub knx_bindings: Vec<KnxBinding>,
    pub logic: LogicDocument,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct StoredAutomation {
    #[serde(default)]
    revision: u16,
    #[serde(flatten)]
    document: AutomationDocument,
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
pub struct LogicDocument {
    pub source: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AutomationEnvelope {
    pub document: AutomationDocument,
    pub revision: u64,
    pub active_structural_revision: u64,
    pub saved_structural_revision: u64,
    pub active_logic_revision: u64,
    pub saved_logic_revision: u64,
    pub restart_required: bool,
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

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct WebConfig {
    pub listen_ip: IpAddr,
    pub listen_port: u16,
}

impl WebConfig {
    pub fn new(listen_ip: IpAddr, listen_port: u16) -> Result<Self, ConfigError> {
        if listen_port == 0 {
            return Err(field("web.listen_port", "must be in range 1..=65535"));
        }
        Ok(Self {
            listen_ip,
            listen_port,
        })
    }

    pub fn socket_addr(self) -> SocketAddr {
        SocketAddr::new(self.listen_ip, self.listen_port)
    }
}

#[derive(Debug, Clone)]
pub struct ConnectionConfig {
    pub gateway_ip: IpAddr,
    pub gateway_port: u16,
    pub local_ip: Option<IpAddr>,
}

#[derive(Debug, Clone)]
pub struct BridgeConfig {
    pub python: PathBuf,
}

#[derive(Debug, Clone, Copy)]
pub struct LoggingConfig {
    pub level: LevelFilter,
    pub bridge_level: LevelFilter,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    knx: RawKnxConfig,
    bridge: RawBridgeConfig,
    logging: RawLoggingConfig,
    web: RawWebConfig,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawKnxConfig {
    connection_type: String,
    gateway_ip: String,
    gateway_port: u32,
    local_ip: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBridgeConfig {
    python: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLoggingConfig {
    level: String,
    bridge_level: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawWebConfig {
    listen_ip: String,
    listen_port: u32,
}

fn field(field: impl Into<String>, message: impl Into<String>) -> ConfigError {
    ConfigError::Field {
        field: field.into(),
        message: message.into(),
    }
}

fn parse_ip(field_name: &str, value: &str) -> Result<IpAddr, ConfigError> {
    value
        .parse()
        .map_err(|error| field(field_name, format!("{error}")))
}

fn parse_level(field_name: &str, value: &str) -> Result<LevelFilter, ConfigError> {
    LevelFilter::from_str(value).map_err(|_| {
        field(
            field_name,
            "must be one of off, error, warn, info, debug, or trace",
        )
    })
}

fn parse_dpt(path: &str, value: &str) -> Result<Dpt, FieldError> {
    let dpt = Dpt::parse(value).map_err(|error| FieldError {
        path: path.to_owned(),
        message: error.to_string(),
    })?;
    if !dpt.is_supported() {
        return Err(FieldError {
            path: path.to_owned(),
            message: "must be 1.001 or 5.001".to_owned(),
        });
    }
    if dpt.to_string() != value {
        return Err(FieldError {
            path: path.to_owned(),
            message: "must use canonical DPT form".to_owned(),
        });
    }
    Ok(dpt)
}

fn endpoint_name(path: &str, value: &str) -> Result<EndpointName, FieldError> {
    value.parse::<EndpointName>().map_err(|error| FieldError {
        path: path.to_owned(),
        message: error.to_string(),
    })
}

const MAX_LOGIC_SOURCE_BYTES: usize = 64 * 1024;

pub fn structural_revision(document: &AutomationDocument) -> u64 {
    let mut structure = document.clone();
    structure.logic.source.clear();
    let bytes = toml::to_string(&structure).unwrap_or_default();
    automation_revision(bytes.as_bytes())
}

/// Validates a complete automation document and constructs its core and
/// desktop-side KNX routing maps.
pub fn build_automation(
    document: AutomationDocument,
) -> Result<AutomationRuntime, Vec<FieldError>> {
    let mut errors = Vec::new();
    let mut endpoint_dpts = HashMap::new();
    let mut endpoints = Vec::new();
    let mut seen_names = HashSet::new();
    for (direction, declarations) in [
        (EndpointDirection::Input, &document.inputs),
        (EndpointDirection::Output, &document.outputs),
    ] {
        for (index, declaration) in declarations.iter().enumerate() {
            let path = match direction {
                EndpointDirection::Input => format!("inputs[{index}]"),
                EndpointDirection::Output => format!("outputs[{index}]"),
            };
            let name = match endpoint_name(&format!("{path}.name"), &declaration.name) {
                Ok(name) => name,
                Err(error) => {
                    errors.push(error);
                    continue;
                }
            };
            if !seen_names.insert(name.clone()) {
                errors.push(FieldError {
                    path: format!("{path}.name"),
                    message: "must be globally unique".to_owned(),
                });
                continue;
            }
            let dpt = match parse_dpt(&format!("{path}.dpt"), &declaration.dpt) {
                Ok(dpt) => dpt,
                Err(error) => {
                    errors.push(error);
                    continue;
                }
            };
            endpoint_dpts.insert(name.clone(), dpt);
            endpoints.push(Endpoint::new(name, direction, dpt));
        }
    }

    let mut address_to_endpoint = HashMap::new();
    let mut endpoint_to_address = HashMap::new();
    for (index, binding) in document.knx_bindings.iter().enumerate() {
        let path = format!("knx_bindings[{index}]");
        let name = match endpoint_name(&format!("{path}.endpoint"), &binding.endpoint) {
            Ok(name) => name,
            Err(error) => {
                errors.push(error);
                continue;
            }
        };
        let Some(&dpt) = endpoint_dpts.get(&name) else {
            errors.push(FieldError {
                path: format!("{path}.endpoint"),
                message: "must reference an existing endpoint".to_owned(),
            });
            continue;
        };
        let direction = endpoints
            .iter()
            .find(|endpoint| endpoint.name == name)
            .map(|endpoint| endpoint.direction)
            .expect("declared endpoint exists");
        let address = match GroupAddress::parse(&binding.group_address) {
            Ok(address) if address.to_string() == binding.group_address => address,
            Ok(_) => {
                errors.push(FieldError {
                    path: format!("{path}.group_address"),
                    message: "must use canonical main/middle/subgroup form".to_owned(),
                });
                continue;
            }
            Err(error) => {
                errors.push(FieldError {
                    path: format!("{path}.group_address"),
                    message: error.to_string(),
                });
                continue;
            }
        };
        if endpoint_to_address.contains_key(&name) {
            errors.push(FieldError {
                path: format!("{path}.endpoint"),
                message: "must have exactly one KNX binding".to_owned(),
            });
            continue;
        }
        if address_to_endpoint.contains_key(&address) {
            errors.push(FieldError {
                path: format!("{path}.group_address"),
                message: "must be globally unique".to_owned(),
            });
            continue;
        }
        endpoint_to_address.insert(name.clone(), address);
        address_to_endpoint.insert(
            address,
            BindingRuntime {
                endpoint: name,
                direction,
                dpt,
                address,
            },
        );
    }
    for endpoint in &endpoints {
        if !endpoint_to_address.contains_key(&endpoint.name) {
            let list = match endpoint.direction {
                EndpointDirection::Input => "inputs",
                EndpointDirection::Output => "outputs",
            };
            errors.push(FieldError {
                path: format!("{list}.name"),
                message: format!(
                    "endpoint {} must have exactly one KNX binding",
                    endpoint.name
                ),
            });
        }
    }

    if document.logic.source.is_empty() {
        errors.push(FieldError {
            path: "logic.source".to_owned(),
            message: "must not be empty".to_owned(),
        });
    } else if document.logic.source.len() > MAX_LOGIC_SOURCE_BYTES {
        errors.push(FieldError {
            path: "logic.source".to_owned(),
            message: "must not exceed 65536 bytes".to_owned(),
        });
    }
    // EngineConfig performs source loading and handler discovery in the core.
    let engine_config = EngineConfig::new(endpoints, document.logic.source.clone());
    if errors.is_empty()
        && let Err(error) = engine_config.validate()
    {
        errors.push(FieldError {
            path: "logic.source".to_owned(),
            message: error.to_string(),
        });
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(AutomationRuntime {
        structural_revision: structural_revision(&document),
        document_revision: 0,
        document,
        engine_config,
        address_to_endpoint,
        endpoint_to_address,
        endpoint_dpts,
    })
}

pub fn automation_revision(source: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in source {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

pub fn load_automation(path: &Path) -> Result<(AutomationDocument, u16), AutomationFileError> {
    let source = fs::read(path).map_err(|source| AutomationFileError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let text = String::from_utf8_lossy(&source);
    let stored = toml::from_str::<StoredAutomation>(&text).map_err(AutomationFileError::Toml)?;
    build_automation(stored.document.clone()).map_err(AutomationFileError::Invalid)?;
    Ok((stored.document, stored.revision))
}

pub fn serialize_automation(
    document: &AutomationDocument,
    revision: u16,
) -> Result<Vec<u8>, String> {
    toml::to_string_pretty(&StoredAutomation {
        revision,
        document: document.clone(),
    })
    .map(|text| text.into_bytes())
    .map_err(|error| error.to_string())
}

pub fn load_config(
    config_path: &Path,
    automation_path: &Path,
) -> Result<RuntimeConfig, ConfigError> {
    let source = fs::read_to_string(config_path).map_err(|source| ConfigError::Read {
        path: config_path.to_path_buf(),
        source,
    })?;
    let raw: RawConfig = toml::from_str(&source)?;
    let automation_source =
        fs::read(automation_path).map_err(|source| ConfigError::AutomationRead {
            path: automation_path.to_path_buf(),
            source,
        })?;
    let automation_text = String::from_utf8_lossy(&automation_source);
    let stored = toml::from_str::<StoredAutomation>(&automation_text)
        .map_err(ConfigError::AutomationToml)?;
    let mut automation =
        build_automation(stored.document).map_err(ConfigError::AutomationInvalid)?;
    automation.document_revision = u64::from(stored.revision);
    if raw.knx.connection_type != "tunneling" {
        return Err(field("knx.connection_type", "must be 'tunneling'"));
    }
    if raw.knx.gateway_port == 0 || raw.knx.gateway_port > u16::MAX as u32 {
        return Err(field("knx.gateway_port", "must be in range 1..=65535"));
    }
    let gateway_ip = parse_ip("knx.gateway_ip", &raw.knx.gateway_ip)?;
    let local_ip = raw
        .knx
        .local_ip
        .as_deref()
        .map(|value| parse_ip("knx.local_ip", value))
        .transpose()?;
    if raw.bridge.python.is_empty() {
        return Err(field("bridge.python", "must not be empty"));
    }
    let python = PathBuf::from(&raw.bridge.python);
    if !python.is_file() {
        return Err(field(
            "bridge.python",
            format!("executable does not exist: {}", python.display()),
        ));
    }
    let listen_ip = parse_ip("web.listen_ip", &raw.web.listen_ip)?;
    if raw.web.listen_port == 0 || raw.web.listen_port > u16::MAX as u32 {
        return Err(field("web.listen_port", "must be in range 1..=65535"));
    }
    Ok(RuntimeConfig {
        config_path: config_path.to_path_buf(),
        automation_path: automation_path.to_path_buf(),
        automation,
        automation_revision: u64::from(stored.revision),
        connection: ConnectionConfig {
            gateway_ip,
            gateway_port: raw.knx.gateway_port as u16,
            local_ip,
        },
        bridge: BridgeConfig { python },
        logging: LoggingConfig {
            level: parse_level("logging.level", &raw.logging.level)?,
            bridge_level: parse_level("logging.bridge_level", &raw.logging.bridge_level)?,
        },
        web: WebConfig {
            listen_ip,
            listen_port: raw.web.listen_port as u16,
        },
    })
}

// ---------------------------------------------------------------------------
// Version-one NDJSON protocol

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DptMessage {
    pub major: u16,
    pub subtype: u16,
}

impl DptMessage {
    fn from_core(dpt: Dpt) -> Self {
        Self {
            major: dpt.major,
            subtype: dpt.subtype,
        }
    }
    fn core(&self, field_name: &'static str) -> Result<Dpt, ProtocolError> {
        let dpt = Dpt::new(self.major, self.subtype)
            .map_err(|error| ProtocolError::Field(field_name, error.to_string()))?;
        if !dpt.is_supported() {
            return Err(ProtocolError::Field(
                field_name,
                "must be 1.001 or 5.001".to_owned(),
            ));
        }
        Ok(dpt)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoolValueMessage {
    pub kind: String,
    pub value: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PercentValueMessage {
    pub kind: String,
    pub value: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ValueMessage {
    Bool(BoolValueMessage),
    Percent(PercentValueMessage),
}

impl ValueMessage {
    fn from_core(value: TypedValue) -> Self {
        match value.value {
            Value::Bool(value) => Self::Bool(BoolValueMessage {
                kind: "bool".to_owned(),
                value,
            }),
            Value::Percent(value) => Self::Percent(PercentValueMessage {
                kind: "percent".to_owned(),
                value,
            }),
        }
    }
    fn core(&self, dpt: Dpt, field_name: &'static str) -> Result<TypedValue, ProtocolError> {
        let value = match self {
            Self::Bool(value) if value.kind == "bool" => Value::Bool(value.value),
            Self::Percent(value) if value.kind == "percent" => Value::Percent(value.value),
            Self::Bool(_) => {
                return Err(ProtocolError::Field(
                    "value.kind",
                    "must be 'bool'".to_owned(),
                ));
            }
            Self::Percent(_) => {
                return Err(ProtocolError::Field(
                    "value.kind",
                    "must be 'percent'".to_owned(),
                ));
            }
        };
        TypedValue::new(dpt, value)
            .map_err(|error| ProtocolError::Field(field_name, error.to_string()))
    }
}

fn simulation_value(value: &ValueMessage, dpt: Dpt, path: &str) -> Result<TypedValue, FieldError> {
    value.core(dpt, "value").map_err(|error| FieldError {
        path: path.to_owned(),
        message: error.to_string(),
    })
}

/// Converts the browser wire representation into the core-owned scenario.
/// Endpoint and value checks that depend on the active configuration are kept
/// here, before the immutable core operation is called.
pub(crate) fn simulation_scenario(
    payload: SimulationPayload,
    automation: &AutomationRuntime,
) -> Result<SimulationScenario, Vec<FieldError>> {
    let mut errors = Vec::new();
    let trigger_endpoint = match endpoint_name("trigger.endpoint", &payload.trigger.endpoint) {
        Ok(endpoint) => Some(endpoint),
        Err(error) => {
            errors.push(error);
            None
        }
    };
    let trigger_dpt = trigger_endpoint
        .as_ref()
        .and_then(|endpoint| automation.endpoint_dpts.get(endpoint).copied());
    if trigger_endpoint.is_some() && trigger_dpt.is_none() {
        errors.push(FieldError {
            path: "trigger.endpoint".to_owned(),
            message: "must reference an existing input endpoint".to_owned(),
        });
    }
    let trigger_value = trigger_dpt.and_then(|dpt| {
        simulation_value(&payload.trigger.value, dpt, "trigger.value")
            .map_err(|error| errors.push(error))
            .ok()
    });
    let previous = match (trigger_dpt, payload.trigger.previous.as_ref()) {
        (Some(dpt), Some(value)) => simulation_value(value, dpt, "trigger.previous")
            .map_err(|error| errors.push(error))
            .ok(),
        (_, None) => None,
        (None, Some(_)) => None,
    };

    let inputs = payload
        .inputs
        .into_iter()
        .enumerate()
        .filter_map(|(index, input)| {
            let path = format!("inputs[{index}]");
            let endpoint = match endpoint_name(&format!("{path}.endpoint"), &input.endpoint) {
                Ok(endpoint) => Some(endpoint),
                Err(error) => {
                    errors.push(error);
                    None
                }
            };
            let dpt = endpoint
                .as_ref()
                .and_then(|endpoint| automation.endpoint_dpts.get(endpoint).copied());
            if endpoint.is_some() && dpt.is_none() {
                errors.push(FieldError {
                    path: format!("{path}.endpoint"),
                    message: "must reference an existing input endpoint".to_owned(),
                });
            }
            let value = match (dpt, input.value.as_ref()) {
                (Some(dpt), Some(value)) => simulation_value(value, dpt, &format!("{path}.value"))
                    .map_err(|error| errors.push(error))
                    .ok(),
                (_, None) => None,
                (None, Some(_)) => None,
            };
            let endpoint = endpoint?;
            Some(SimulationInput {
                endpoint,
                value,
                valid: input.valid,
                age_ms: input.age_ms,
            })
        })
        .collect::<Vec<_>>();

    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(SimulationScenario {
        trigger: SimulationTrigger {
            endpoint: trigger_endpoint.expect("validated trigger endpoint"),
            value: trigger_value.expect("validated trigger value"),
            previous,
        },
        inputs,
    })
}

pub(crate) fn simulation_error_fields(
    error: &SimulationError,
    payload: &SimulationPayload,
) -> Vec<FieldError> {
    let endpoint_path = |endpoint: &EndpointName| {
        if payload.trigger.endpoint == endpoint.as_str() {
            "trigger.endpoint".to_owned()
        } else {
            payload
                .inputs
                .iter()
                .enumerate()
                .find(|(_, input)| input.endpoint == endpoint.as_str())
                .map(|(index, _)| format!("inputs[{index}].endpoint"))
                .unwrap_or_else(|| "inputs".to_owned())
        }
    };
    let input_field = |endpoint: &EndpointName, field_name: &str| {
        payload
            .inputs
            .iter()
            .enumerate()
            .find(|(_, input)| input.endpoint == endpoint.as_str())
            .map(|(index, _)| format!("inputs[{index}].{field_name}"))
            .unwrap_or_else(|| "inputs".to_owned())
    };
    let path = match error {
        SimulationError::UnknownEndpoint(endpoint)
        | SimulationError::EndpointNotInput { endpoint, .. } => endpoint_path(endpoint),
        SimulationError::DuplicateInput(endpoint) | SimulationError::MissingInput(endpoint) => {
            input_field(endpoint, "endpoint")
        }
        SimulationError::DptMismatch { endpoint, .. } => {
            if payload.trigger.endpoint == endpoint.as_str() {
                "trigger.value".to_owned()
            } else {
                input_field(endpoint, "value")
            }
        }
        SimulationError::InvalidValue(_) => "inputs".to_owned(),
        SimulationError::TriggerValueMismatch { .. } => "trigger.value".to_owned(),
        SimulationError::MissingValue(endpoint) | SimulationError::UnexpectedValue(endpoint) => {
            input_field(endpoint, "value")
        }
        SimulationError::MissingAge(endpoint) | SimulationError::UnexpectedAge(endpoint) => {
            input_field(endpoint, "age_ms")
        }
        SimulationError::TriggerAgeMismatch { endpoint, .. } => input_field(endpoint, "age_ms"),
    };
    vec![FieldError {
        path,
        message: error.to_string(),
    }]
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroupAddressDpt {
    pub address: String,
    pub dpt: DptMessage,
}

impl GroupAddressDpt {
    fn validate(&self, field_name: &'static str) -> Result<GroupAddress, ProtocolError> {
        let address = GroupAddress::parse(&self.address)
            .map_err(|error| ProtocolError::Field(field_name, error.to_string()))?;
        if address.to_string() != self.address {
            return Err(ProtocolError::Field(
                field_name,
                "must use canonical main/middle/subgroup form".to_owned(),
            ));
        }
        self.dpt.core("dpt")?;
        Ok(address)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TunnelingConnection {
    #[serde(rename = "type")]
    pub connection_type: String,
    pub gateway_ip: String,
    pub gateway_port: u16,
    pub local_ip: Option<String>,
}

impl TunnelingConnection {
    fn validate(&self) -> Result<(), ProtocolError> {
        if self.connection_type != "tunneling" {
            return Err(ProtocolError::Field(
                "connection.type",
                "must be 'tunneling'".to_owned(),
            ));
        }
        self.gateway_ip
            .parse::<IpAddr>()
            .map_err(|error| ProtocolError::Field("connection.gateway_ip", error.to_string()))?;
        if self.gateway_port == 0 {
            return Err(ProtocolError::Field(
                "connection.gateway_port",
                "must be greater than zero".to_owned(),
            ));
        }
        if let Some(local_ip) = &self.local_ip {
            local_ip
                .parse::<IpAddr>()
                .map_err(|error| ProtocolError::Field("connection.local_ip", error.to_string()))?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BridgeHello {
    pub v: u64,
    #[serde(rename = "type")]
    pub message_type: String,
    pub bridge: String,
    pub bridge_version: String,
    pub xknx_version: String,
}

impl BridgeHello {
    fn validate(&self) -> Result<(), ProtocolError> {
        if self.bridge != "xknx" || self.bridge_version.is_empty() || self.xknx_version.is_empty() {
            return Err(ProtocolError::Field(
                "bridge",
                "invalid bridge hello".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Configure {
    pub v: u64,
    #[serde(rename = "type")]
    pub message_type: String,
    pub connection: TunnelingConnection,
    pub group_addresses: Vec<GroupAddressDpt>,
}

impl Configure {
    fn validate(&self) -> Result<(), ProtocolError> {
        self.connection.validate()?;
        if self.group_addresses.is_empty() {
            return Err(ProtocolError::Field(
                "group_addresses",
                "must not be empty".to_owned(),
            ));
        }
        let mut addresses = HashSet::new();
        for (index, entry) in self.group_addresses.iter().enumerate() {
            let address = entry.validate("group_addresses")?;
            if !addresses.insert(address) {
                return Err(ProtocolError::Field(
                    "group_addresses",
                    format!("duplicate address at index {index}"),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ready {
    pub v: u64,
    #[serde(rename = "type")]
    pub message_type: String,
    pub transport: String,
    pub gateway: String,
}

impl Ready {
    fn validate(&self) -> Result<(), ProtocolError> {
        if self.transport != "knxip_tunneling" {
            return Err(ProtocolError::Field(
                "transport",
                "must be 'knxip_tunneling'".to_owned(),
            ));
        }
        self.gateway
            .parse::<IpAddr>()
            .map_err(|error| ProtocolError::Field("gateway", error.to_string()))?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnxEvent {
    pub v: u64,
    #[serde(rename = "type")]
    pub message_type: String,
    pub source: Option<String>,
    pub destination: String,
    pub service: String,
    pub dpt: DptMessage,
    pub value: Option<ValueMessage>,
}

impl KnxEvent {
    fn typed_value(&self) -> Result<Option<TypedValue>, ProtocolError> {
        let dpt = self.dpt.core("dpt")?;
        self.value
            .as_ref()
            .map(|value| value.core(dpt, "value"))
            .transpose()
    }
    fn validate(&self) -> Result<(), ProtocolError> {
        let destination = GroupAddress::parse(&self.destination)
            .map_err(|error| ProtocolError::Field("destination", error.to_string()))?;
        if destination.to_string() != self.destination {
            return Err(ProtocolError::Field(
                "destination",
                "must use canonical main/middle/subgroup form".to_owned(),
            ));
        }
        match self.service.as_str() {
            "group_value_read" if self.value.is_none() => {}
            "group_value_write" | "group_value_response" if self.value.is_some() => {
                self.typed_value()?;
            }
            "group_value_read" => {
                return Err(ProtocolError::Field(
                    "value",
                    "group_value_read must not carry a value".to_owned(),
                ));
            }
            _ => {
                return Err(ProtocolError::Field(
                    "service",
                    "unsupported KNX group service or missing value".to_owned(),
                ));
            }
        }
        Ok(())
    }
    fn to_input_event(&self, endpoint: EndpointName) -> Result<InputEvent, ProtocolError> {
        if self.service != "group_value_write" {
            return Err(ProtocolError::Field(
                "service",
                "only group_value_write drives logic".to_owned(),
            ));
        }
        let value = self.typed_value()?.ok_or_else(|| {
            ProtocolError::Field("value", "required for group_value_write".to_owned())
        })?;
        Ok(InputEvent::new(endpoint, value))
    }

    fn to_input_observation(
        &self,
        endpoint: EndpointName,
    ) -> Result<InputObservation, ProtocolError> {
        if self.service != "group_value_response" {
            return Err(ProtocolError::Field(
                "service",
                "only group_value_response records a passive observation".to_owned(),
            ));
        }
        let value = self.typed_value()?.ok_or_else(|| {
            ProtocolError::Field("value", "required for group_value_response".to_owned())
        })?;
        Ok(InputObservation::new(endpoint, value))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnxWrite {
    pub v: u64,
    #[serde(rename = "type")]
    pub message_type: String,
    pub request_id: u64,
    pub destination: String,
    pub dpt: DptMessage,
    pub value: ValueMessage,
}

impl KnxWrite {
    fn validate(&self) -> Result<(), ProtocolError> {
        let destination = GroupAddress::parse(&self.destination)
            .map_err(|error| ProtocolError::Field("destination", error.to_string()))?;
        if destination.to_string() != self.destination {
            return Err(ProtocolError::Field(
                "destination",
                "must use canonical main/middle/subgroup form".to_owned(),
            ));
        }
        let dpt = self.dpt.core("dpt")?;
        self.value.core(dpt, "value")?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandResult {
    pub v: u64,
    #[serde(rename = "type")]
    pub message_type: String,
    pub request_id: u64,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl CommandResult {
    fn validate(&self) -> Result<(), ProtocolError> {
        if !self.ok && self.error.as_deref().is_none_or(str::is_empty) {
            return Err(ProtocolError::Field(
                "error",
                "is required when ok is false".to_owned(),
            ));
        }
        if self.ok && self.error.is_some() {
            return Err(ProtocolError::Field(
                "error",
                "must be omitted when ok is true".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fatal {
    pub v: u64,
    #[serde(rename = "type")]
    pub message_type: String,
    pub code: String,
    pub message: String,
}

impl Fatal {
    fn validate(&self) -> Result<(), ProtocolError> {
        if self.code.is_empty() || self.message.is_empty() {
            return Err(ProtocolError::Field(
                "code/message",
                "must not be empty".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Shutdown {
    pub v: u64,
    #[serde(rename = "type")]
    pub message_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    BridgeHello(BridgeHello),
    Configure(Configure),
    Ready(Ready),
    KnxEvent(KnxEvent),
    KnxWrite(KnxWrite),
    CommandResult(CommandResult),
    Fatal(Fatal),
    Shutdown(Shutdown),
}

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("invalid protocol JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("protocol message must be an object with v and type")]
    Envelope,
    #[error("bridge protocol unsupported expected={PROTOCOL_VERSION} received={0}")]
    Version(u64),
    #[error("unsupported protocol message type: {0}")]
    MessageType(String),
    #[error("invalid protocol field {0}: {1}")]
    Field(&'static str, String),
}

#[derive(Debug, Deserialize)]
struct Envelope {
    v: u64,
    #[serde(rename = "type")]
    message_type: String,
}

fn decode<T: for<'de> Deserialize<'de>>(line: &str) -> Result<T, ProtocolError> {
    serde_json::from_str(line).map_err(ProtocolError::Json)
}

pub fn parse_message(line: &str) -> Result<Message, ProtocolError> {
    if line.trim().is_empty() {
        return Err(ProtocolError::Envelope);
    }
    let envelope: Envelope = decode(line)?;
    if envelope.v != PROTOCOL_VERSION {
        return Err(ProtocolError::Version(envelope.v));
    }
    Ok(match envelope.message_type.as_str() {
        "bridge_hello" => {
            let message: BridgeHello = decode(line)?;
            message.validate()?;
            Message::BridgeHello(message)
        }
        "configure" => {
            let message: Configure = decode(line)?;
            message.validate()?;
            Message::Configure(message)
        }
        "ready" => {
            let message: Ready = decode(line)?;
            message.validate()?;
            Message::Ready(message)
        }
        "knx_event" => {
            let message: KnxEvent = decode(line)?;
            message.validate()?;
            Message::KnxEvent(message)
        }
        "knx_write" => {
            let message: KnxWrite = decode(line)?;
            message.validate()?;
            Message::KnxWrite(message)
        }
        "command_result" => {
            let message: CommandResult = decode(line)?;
            message.validate()?;
            Message::CommandResult(message)
        }
        "fatal" => {
            let message: Fatal = decode(line)?;
            message.validate()?;
            Message::Fatal(message)
        }
        "shutdown" => Message::Shutdown(decode(line)?),
        other => return Err(ProtocolError::MessageType(other.to_owned())),
    })
}

pub fn encode_message(message: &Message) -> Result<String, ProtocolError> {
    Ok(match message {
        Message::BridgeHello(message) => serde_json::to_string(message),
        Message::Configure(message) => serde_json::to_string(message),
        Message::Ready(message) => serde_json::to_string(message),
        Message::KnxEvent(message) => serde_json::to_string(message),
        Message::KnxWrite(message) => serde_json::to_string(message),
        Message::CommandResult(message) => serde_json::to_string(message),
        Message::Fatal(message) => serde_json::to_string(message),
        Message::Shutdown(message) => serde_json::to_string(message),
    }?)
}

// ---------------------------------------------------------------------------
// Child-process runtime

#[derive(Debug, Clone)]
pub struct BridgeCommand {
    pub executable: PathBuf,
    pub args: Vec<OsString>,
}

impl BridgeCommand {
    pub fn new(executable: impl Into<PathBuf>, args: Vec<OsString>) -> Self {
        Self {
            executable: executable.into(),
            args,
        }
    }
}

#[derive(Debug, Error)]
pub enum HostError {
    #[error("failed to start XKNX bridge `{path}`: {source}")]
    Start { path: PathBuf, source: io::Error },
    #[error("bridge protocol error: {0}")]
    Protocol(#[from] ProtocolError),
    #[error("bridge stdout reached EOF")]
    StdoutEof,
    #[error("KNX bridge fatal code={code}: {message}")]
    BridgeFatal { code: String, message: String },
    #[error("KNX bridge exited unexpectedly status={status}")]
    BridgeExited { status: String },
    #[error("bridge I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("bridge command result has unknown request_id={0}")]
    UnknownRequest(u64),
    #[error("dashboard startup failed: {0}")]
    Web(#[from] WebError),
}

pub async fn run(config: RuntimeConfig) -> Result<(), HostError> {
    run_with_bridge(
        config.clone(),
        BridgeCommand::new(
            config.bridge.python,
            vec![OsString::from("-m"), OsString::from("logiksmith_xknx")],
        ),
    )
    .await
}

pub async fn run_with_bridge(
    config: RuntimeConfig,
    bridge_command: BridgeCommand,
) -> Result<(), HostError> {
    let store = DiagnosticStore::new(
        &config.automation,
        config.automation_path.clone(),
        config.automation_revision,
    );
    init_logging(config.logging, store.clone());
    let mut engine = Engine::try_new(config.automation.engine_config.clone()).map_err(|error| {
        HostError::Protocol(ProtocolError::Field("automation", error.to_string()))
    })?;
    let (activation_sender, activation_receiver) = mpsc::channel(8);
    let (simulation_sender, simulation_receiver) = mpsc::channel(8);
    let web_server = web::start_web_server_with_runtime(
        store.clone(),
        config.web,
        activation_sender,
        simulation_sender,
    )
    .await?;
    let mut child = match Command::new(&bridge_command.executable)
        .args(&bridge_command.args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(source) => {
            web_server.shutdown().await;
            return Err(HostError::Start {
                path: bridge_command.executable,
                source,
            });
        }
    };
    let mut stdin = child.stdin.take().ok_or_else(|| HostError::Start {
        path: bridge_command.executable.clone(),
        source: io::Error::other("bridge stdin was not piped"),
    })?;
    let stdout = child.stdout.take().ok_or_else(|| HostError::Start {
        path: bridge_command.executable.clone(),
        source: io::Error::other("bridge stdout was not piped"),
    })?;
    if let Some(stderr) = child.stderr.take() {
        tokio::spawn(forward_bridge_stderr(stderr));
    }
    let mut reader = BufReader::new(stdout);
    let result = run_session(
        &config,
        &store,
        &mut engine,
        &mut child,
        &mut stdin,
        &mut reader,
        activation_receiver,
        simulation_receiver,
    )
    .await;
    if result.is_err() {
        let _ = send_message(&mut stdin, &shutdown_message()).await;
        terminate_child(&mut child).await;
    }
    web_server.shutdown().await;
    result
}

async fn forward_bridge_stderr<R: AsyncRead + Unpin>(stderr: R) {
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        tracing::debug!(target: "bridge.xknx", "{line}");
    }
}

async fn run_session(
    config: &RuntimeConfig,
    store: &DiagnosticStore,
    engine: &mut Engine,
    child: &mut Child,
    stdin: &mut ChildStdin,
    reader: &mut BufReader<ChildStdout>,
    mut activations: mpsc::Receiver<ActivationRequest>,
    mut simulations: mpsc::Receiver<SimulationRequest>,
) -> Result<(), HostError> {
    let hello = match read_message(reader).await? {
        Message::BridgeHello(hello) => hello,
        Message::Fatal(fatal) => {
            return Err(HostError::BridgeFatal {
                code: fatal.code,
                message: fatal.message,
            });
        }
        _ => {
            return Err(ProtocolError::Field(
                "startup",
                "expected bridge_hello before configure".to_owned(),
            )
            .into());
        }
    };
    tracing::info!(target: "logiksmith", bridge = %hello.bridge, bridge_version = %hello.bridge_version, xknx_version = %hello.xknx_version, "bridge hello");
    store.set_connection(ConnectionState::Connecting);
    send_message(stdin, &configure_message(config)).await?;
    let ready = match read_message(reader).await? {
        Message::Ready(ready) => ready,
        Message::Fatal(fatal) => {
            return Err(HostError::BridgeFatal {
                code: fatal.code,
                message: fatal.message,
            });
        }
        _ => {
            return Err(ProtocolError::Field(
                "startup",
                "expected ready after configure".to_owned(),
            )
            .into());
        }
    };
    store.set_connection(ConnectionState::Connected);
    tracing::info!(target: "logiksmith", gateway = %ready.gateway, "KNX connected");
    let interrupt = signal::ctrl_c();
    tokio::pin!(interrupt);
    let mut line = String::new();
    let mut next_request_id = 1u64;
    let mut pending = HashSet::new();
    let mut active_logic_revision = config.automation.document_revision;
    loop {
        tokio::select! {
            read = reader.read_line(&mut line) => {
                let bytes = read?;
                if bytes == 0 { let status = child.wait().await?; return Err(HostError::BridgeExited { status: format_status(status) }); }
                let message = parse_message(line.trim_end_matches(['\r', '\n']))?;
                line.clear();
                match message {
                    Message::KnxEvent(event) => {
                        let destination = GroupAddress::parse(&event.destination).map_err(|error| ProtocolError::Field("destination", error.to_string()))?;
                        let logical_endpoint = config.automation.address_to_endpoint.get(&destination).map(|binding| binding.endpoint.clone());
                        store.record_telegram(TelegramRecord::from_event(&event, logical_endpoint.as_ref()));
                        let Some(binding) = config.automation.address_to_endpoint.get(&destination) else { continue };
                        if binding.direction != EndpointDirection::Input || event.value.is_none() { continue; }
                        if event.service == "group_value_write" {
                            let input = match event.to_input_event(binding.endpoint.clone()) { Ok(input) => input, Err(error) => { tracing::warn!(target: "logiksmith", error = %error, "ignoring invalid logical input event"); continue; } };
                            let now = store.now();
                            let started = Instant::now();
                            let execution = match engine.process_input(input, now) {
                                Ok(execution) => execution,
                                Err(error) => {
                                    tracing::warn!(target: "logiksmith", error = %error, "ignoring invalid logical input event");
                                    continue;
                                }
                            };
                            let duration_us = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
                            store.record_execution(&execution, duration_us, &config.automation);
                            if let Ok(effects) = &execution.outcome {
                                dispatch_effects(store, stdin, &config.automation, effects.clone(), &mut next_request_id, &mut pending).await?;
                            }
                        } else if event.service == "group_value_response"
                            && let Ok(observation) = event.to_input_observation(binding.endpoint.clone())
                            && let Err(error) = engine.observe_input(observation, store.now()) { tracing::warn!(target: "logiksmith", error = %error, "ignoring invalid passive input observation"); }
                    }
                    Message::CommandResult(result) => {
                        if !pending.remove(&result.request_id) { return Err(HostError::UnknownRequest(result.request_id)); }
                        store.record_write_result(result.request_id, result.ok, result.error.clone());
                    }
                    Message::Fatal(fatal) => return Err(HostError::BridgeFatal { code: fatal.code, message: fatal.message }),
                    _ => return Err(ProtocolError::Field("runtime", "unexpected bridge message".to_owned()).into()),
                }
            }
            Some(request) = activations.recv() => {
                let source = request.source;
                let result = engine.replace_logic(source.clone(), request.logic_hash).map(|_| ()).map_err(|error| error.to_string());
                if result.is_ok() {
                    active_logic_revision = request.document_revision;
                    store.set_active_logic(request.document_revision, source);
                }
                let _ = request.reply.send(result);
            }
            Some(request) = simulations.recv() => {
                let SimulationRequest { payload, reply } = request;
                let outcome = if payload.expected_logic_revision != active_logic_revision {
                    SimulationOutcome::Conflict { current_revision: active_logic_revision }
                } else {
                    match simulation_scenario(payload.clone(), &config.automation) {
                        Err(errors) => SimulationOutcome::Invalid(errors),
                        Ok(scenario) => {
                            let started = Instant::now();
                            let execution = engine.simulate_input(scenario);
                            let duration_us = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
                            match execution {
                                Ok(execution) => SimulationOutcome::Complete(
                                    diagnostics::simulation_response(
                                        &execution,
                                        duration_us,
                                        active_logic_revision,
                                        &config.automation,
                                    ),
                                ),
                                Err(error) => SimulationOutcome::Invalid(
                                    simulation_error_fields(&error, &payload),
                                ),
                            }
                        }
                    }
                };
                let _ = reply.send(outcome);
            }
            signal = &mut interrupt => {
                signal?;
                let _ = send_message(stdin, &shutdown_message()).await;
                terminate_child(child).await;
                return Ok(());
            }
        }
    }
}

fn configure_message(config: &RuntimeConfig) -> Message {
    let mut group_addresses: Vec<_> = config
        .automation
        .address_to_endpoint
        .values()
        .map(|binding| GroupAddressDpt {
            address: binding.address.to_string(),
            dpt: DptMessage::from_core(binding.dpt),
        })
        .collect();
    group_addresses.sort_by(|left, right| left.address.cmp(&right.address));
    Message::Configure(Configure {
        v: PROTOCOL_VERSION,
        message_type: "configure".to_owned(),
        connection: TunnelingConnection {
            connection_type: "tunneling".to_owned(),
            gateway_ip: config.connection.gateway_ip.to_string(),
            gateway_port: config.connection.gateway_port,
            local_ip: config.connection.local_ip.map(|ip| ip.to_string()),
        },
        group_addresses,
    })
}

fn shutdown_message() -> Message {
    Message::Shutdown(Shutdown {
        v: PROTOCOL_VERSION,
        message_type: "shutdown".to_owned(),
    })
}

async fn dispatch_effects(
    store: &DiagnosticStore,
    stdin: &mut ChildStdin,
    automation: &AutomationRuntime,
    effects: Vec<Effect>,
    next_request_id: &mut u64,
    pending: &mut HashSet<u64>,
) -> Result<(), HostError> {
    for effect in effects {
        let Effect::SetOutput { endpoint, value } = effect;
        let Some(destination) = automation.endpoint_to_address.get(&endpoint).copied() else {
            tracing::error!(target: "logiksmith", endpoint = %endpoint, "core returned an unresolved output effect");
            continue;
        };
        let request_id = *next_request_id;
        *next_request_id = next_request_id.checked_add(1).ok_or_else(|| {
            HostError::Protocol(ProtocolError::Field("request_id", "exhausted".to_owned()))
        })?;
        let dpt = automation
            .endpoint_dpts
            .get(&endpoint)
            .copied()
            .ok_or_else(|| {
                HostError::Protocol(ProtocolError::Field(
                    "output",
                    "missing endpoint DPT".to_owned(),
                ))
            })?;
        if value.dpt != dpt {
            tracing::error!(target: "logiksmith", endpoint = %endpoint, "core returned an output value with the wrong DPT");
            continue;
        }
        pending.insert(request_id);
        store.record_write_requested(request_id, endpoint.clone(), destination, dpt, value);
        let message = Message::KnxWrite(KnxWrite {
            v: PROTOCOL_VERSION,
            message_type: "knx_write".to_owned(),
            request_id,
            destination: destination.to_string(),
            dpt: DptMessage::from_core(dpt),
            value: ValueMessage::from_core(value),
        });
        if let Err(error) = send_message(stdin, &message).await {
            pending.remove(&request_id);
            store.record_write_result(request_id, false, Some(error.to_string()));
            return Err(error);
        }
    }
    Ok(())
}

async fn read_message(reader: &mut BufReader<ChildStdout>) -> Result<Message, HostError> {
    let mut line = String::new();
    let bytes = reader.read_line(&mut line).await?;
    if bytes == 0 {
        return Err(HostError::StdoutEof);
    }
    Ok(parse_message(line.trim_end_matches(['\r', '\n']))?)
}

async fn send_message(stdin: &mut ChildStdin, message: &Message) -> Result<(), HostError> {
    stdin.write_all(encode_message(message)?.as_bytes()).await?;
    stdin.write_all(b"\n").await?;
    stdin.flush().await?;
    Ok(())
}

fn init_logging(config: LoggingConfig, store: DiagnosticStore) {
    diagnostics::activate_tracing_store(store);
    let filter = logging_filter(config);
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_target(true)
        .with_filter(filter.clone());
    let diagnostics_layer = diagnostics::tracing_layer().with_filter(filter);
    let _ = tracing_subscriber::registry()
        .with(fmt_layer)
        .with(diagnostics_layer)
        .try_init();
}

fn logging_filter(config: LoggingConfig) -> EnvFilter {
    EnvFilter::builder()
        .with_default_directive(config.level.into())
        .parse_lossy(format!(
            "logiksmith={},bridge.xknx={}",
            config.level, config.bridge_level
        ))
}

async fn terminate_child(child: &mut Child) {
    if time::timeout(Duration::from_millis(250), child.wait())
        .await
        .is_ok()
    {
        return;
    }
    let _ = child.kill().await;
    let _ = child.wait().await;
}

fn format_status(status: std::process::ExitStatus) -> String {
    status
        .code()
        .map_or_else(|| "signal".to_owned(), |code| code.to_string())
}

impl fmt::Display for DptMessage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{:03}", self.major, self.subtype)
    }
}
