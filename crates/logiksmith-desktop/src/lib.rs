//! Tokio desktop host for the platform-independent LogikSmith engine.

use logiksmith_core::{
    Command as CoreCommand, ConfigError as CoreConfigError, Dpt, Engine, EngineConfig,
    GroupAddress, GroupService, IndividualAddress, KnxEvent as CoreKnxEvent, MonotonicMs, Value,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    ffi::OsString,
    fmt, io,
    net::IpAddr,
    path::{Path, PathBuf},
    str::FromStr,
    time::{Duration, Instant},
};
use thiserror::Error;
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    signal,
    time::{self, MissedTickBehavior},
};
use tracing_subscriber::{EnvFilter, filter::LevelFilter};

pub const PROTOCOL_VERSION: u64 = 1;

// ---------------------------------------------------------------------------
// Configuration

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub config_path: PathBuf,
    pub engine: EngineConfig,
    pub connection: ConnectionConfig,
    pub bridge: BridgeConfig,
    pub logging: LoggingConfig,
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

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("cannot read configuration {path}: {source}")]
    Read { path: PathBuf, source: io::Error },
    #[error("invalid TOML configuration: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("invalid configuration field {field}: {message}")]
    Field {
        field: &'static str,
        message: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    knx: RawKnxConfig,
    poc: RawPocConfig,
    bridge: RawBridgeConfig,
    logging: RawLoggingConfig,
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
struct RawPocConfig {
    input_group_address: String,
    input_dpt: String,
    output_group_address: String,
    output_dpt: String,
    off_delay_ms: u64,
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

fn field(field: &'static str, message: impl Into<String>) -> ConfigError {
    ConfigError::Field {
        field,
        message: message.into(),
    }
}

fn parse_ip(field_name: &'static str, value: &str) -> Result<IpAddr, ConfigError> {
    value
        .parse()
        .map_err(|error| field(field_name, format!("{error}")))
}

fn parse_level(field_name: &'static str, value: &str) -> Result<LevelFilter, ConfigError> {
    LevelFilter::from_str(value).map_err(|_| {
        field(
            field_name,
            "must be one of off, error, warn, info, debug, or trace",
        )
    })
}

/// Loads TOML and turns its POC section into the already-validated core
/// configuration. Address and DPT parsing intentionally goes through core.
pub fn load_config(path: &Path) -> Result<RuntimeConfig, ConfigError> {
    let source = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let raw: RawConfig = toml::from_str(&source)?;

    if raw.knx.connection_type != "tunneling" {
        return Err(field(
            "knx.connection_type",
            "must be 'tunneling' for this proof of concept",
        ));
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

    let input_group_address = GroupAddress::parse(&raw.poc.input_group_address)
        .map_err(|error| field("poc.input_group_address", error.to_string()))?;
    let output_group_address = GroupAddress::parse(&raw.poc.output_group_address)
        .map_err(|error| field("poc.output_group_address", error.to_string()))?;
    let input_dpt = Dpt::parse(&raw.poc.input_dpt)
        .map_err(|error| field("poc.input_dpt", error.to_string()))?;
    let output_dpt = Dpt::parse(&raw.poc.output_dpt)
        .map_err(|error| field("poc.output_dpt", error.to_string()))?;
    let engine = EngineConfig {
        input_group_address,
        input_dpt,
        output_group_address,
        output_dpt,
        off_delay_ms: raw.poc.off_delay_ms,
    };
    validate_engine_config(engine)?;

    let python = PathBuf::from(&raw.bridge.python);
    if raw.bridge.python.is_empty() {
        return Err(field("bridge.python", "must not be empty"));
    }
    if !python.is_file() {
        return Err(field(
            "bridge.python",
            format!("executable does not exist: {}", python.display()),
        ));
    }

    Ok(RuntimeConfig {
        config_path: path.to_path_buf(),
        engine,
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
    })
}

fn validate_engine_config(config: EngineConfig) -> Result<(), ConfigError> {
    match Engine::try_new(config) {
        Ok(_) => Ok(()),
        Err(CoreConfigError::SameGroupAddress) => Err(field(
            "poc.input_group_address/poc.output_group_address",
            "must differ",
        )),
        Err(CoreConfigError::UnsupportedInputDpt(dpt)) => {
            Err(field("poc.input_dpt", format!("must be 1.001, got {dpt}")))
        }
        Err(CoreConfigError::UnsupportedOutputDpt(dpt)) => {
            Err(field("poc.output_dpt", format!("must be 1.001, got {dpt}")))
        }
        Err(CoreConfigError::ZeroOffDelay) => {
            Err(field("poc.off_delay_ms", "must be greater than zero"))
        }
        Err(CoreConfigError::OffDelayTooLarge { actual, maximum }) => Err(field(
            "poc.off_delay_ms",
            format!("{actual} exceeds maximum {maximum}"),
        )),
    }
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
    fn bool() -> Self {
        Self {
            major: 1,
            subtype: 1,
        }
    }

    fn core(&self, field_name: &'static str) -> Result<Dpt, ProtocolError> {
        Dpt::new(self.major, self.subtype)
            .map_err(|error| ProtocolError::Field(field_name, error.to_string()))
    }

    fn require_bool(&self, field_name: &'static str) -> Result<(), ProtocolError> {
        if self.major == 1 && self.subtype == 1 {
            Ok(())
        } else {
            Err(ProtocolError::Field(
                field_name,
                format!(
                    "only DPT 1.001 is supported, got {}.{:03}",
                    self.major, self.subtype
                ),
            ))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoolValueMessage {
    pub kind: String,
    pub value: bool,
}

impl BoolValueMessage {
    fn validate(&self) -> Result<(), ProtocolError> {
        if self.kind == "bool" {
            Ok(())
        } else {
            Err(ProtocolError::Field(
                "value.kind",
                "must be 'bool'".to_owned(),
            ))
        }
    }

    fn core(&self) -> Result<Value, ProtocolError> {
        self.validate()?;
        Ok(Value::Bool(self.value))
    }
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
        self.dpt.require_bool("dpt")?;
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
        if self.bridge != "xknx" {
            return Err(ProtocolError::Field("bridge", "must be 'xknx'".to_owned()));
        }
        if self.bridge_version.is_empty() || self.xknx_version.is_empty() {
            return Err(ProtocolError::Field(
                "bridge_version/xknx_version",
                "must not be empty".to_owned(),
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
        if self.group_addresses.len() != 2 {
            return Err(ProtocolError::Field(
                "group_addresses",
                "must contain input then output".to_owned(),
            ));
        }
        let input = self.group_addresses[0].validate("group_addresses[0].address")?;
        let output = self.group_addresses[1].validate("group_addresses[1].address")?;
        if input == output {
            return Err(ProtocolError::Field(
                "group_addresses",
                "input and output group addresses must differ".to_owned(),
            ));
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
    pub value: Option<BoolValueMessage>,
}

impl KnxEvent {
    fn to_core(&self) -> Result<CoreKnxEvent, ProtocolError> {
        let source = self
            .source
            .as_deref()
            .map(|source| {
                let parsed = IndividualAddress::parse(source)
                    .map_err(|error| ProtocolError::Field("source", error.to_string()))?;
                if parsed.to_string() != source {
                    return Err(ProtocolError::Field(
                        "source",
                        "must use canonical area.line.device form".to_owned(),
                    ));
                }
                Ok(parsed)
            })
            .transpose()?;
        let destination = GroupAddress::parse(&self.destination)
            .map_err(|error| ProtocolError::Field("destination", error.to_string()))?;
        if destination.to_string() != self.destination {
            return Err(ProtocolError::Field(
                "destination",
                "must use canonical main/middle/subgroup form".to_owned(),
            ));
        }
        self.dpt.require_bool("dpt")?;
        let (service, value) = match self.service.as_str() {
            "group_value_write" => (GroupService::Write, self.value.as_ref()),
            "group_value_response" => (GroupService::Response, self.value.as_ref()),
            "group_value_read" => (GroupService::Read, None),
            _ => {
                return Err(ProtocolError::Field(
                    "service",
                    "unsupported KNX group service".to_owned(),
                ));
            }
        };
        if self.service == "group_value_read" && self.value.is_some() {
            return Err(ProtocolError::Field(
                "value",
                "group_value_read must not carry a value".to_owned(),
            ));
        }
        if self.service != "group_value_read" && self.value.is_none() {
            return Err(ProtocolError::Field(
                "value",
                "this KNX service must carry a value".to_owned(),
            ));
        }
        let value = value.map(BoolValueMessage::core).transpose()?;
        Ok(CoreKnxEvent {
            source,
            destination,
            service,
            dpt: self.dpt.core("dpt")?,
            value,
        })
    }

    fn validate(&self) -> Result<(), ProtocolError> {
        self.to_core().map(|_| ())
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
    pub value: BoolValueMessage,
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
        self.dpt.require_bool("dpt")?;
        self.value.validate()
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
        if self.error.as_deref().is_some_and(str::is_empty) {
            return Err(ProtocolError::Field(
                "error",
                "must not be empty".to_owned(),
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

/// Parses one strict version-one protocol line. The caller owns framing; the
/// parser rejects malformed JSON, unknown message types, and wrong versions.
pub fn parse_message(line: &str) -> Result<Message, ProtocolError> {
    if line.trim().is_empty() {
        return Err(ProtocolError::Envelope);
    }
    let envelope: Envelope = decode(line)?;
    if envelope.v != PROTOCOL_VERSION {
        return Err(ProtocolError::Version(envelope.v));
    }
    let message = match envelope.message_type.as_str() {
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
    };
    Ok(message)
}

pub fn encode_message(message: &Message) -> Result<String, ProtocolError> {
    let value = match message {
        Message::BridgeHello(message) => serde_json::to_string(message),
        Message::Configure(message) => serde_json::to_string(message),
        Message::Ready(message) => serde_json::to_string(message),
        Message::KnxEvent(message) => serde_json::to_string(message),
        Message::KnxWrite(message) => serde_json::to_string(message),
        Message::CommandResult(message) => serde_json::to_string(message),
        Message::Fatal(message) => serde_json::to_string(message),
        Message::Shutdown(message) => serde_json::to_string(message),
    }?;
    Ok(value)
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

/// Runs the host with an explicit child command. This small seam is also used
/// by the no-network fake-bridge integration test.
pub async fn run_with_bridge(
    config: RuntimeConfig,
    bridge_command: BridgeCommand,
) -> Result<(), HostError> {
    init_logging(config.logging);
    tracing::info!(target: "logiksmith", version = env!("CARGO_PKG_VERSION"), "logiksmith starting");
    tracing::info!(target: "logiksmith", path = %config.config_path.display(), "configuration loaded");
    let mut engine = Engine::try_new(config.engine)
        .map_err(|error| HostError::Protocol(ProtocolError::Field("poc", error.to_string())))?;
    tracing::info!(target: "logiksmith", "core initialized");

    tracing::info!(target: "logiksmith", executable = %bridge_command.executable.display(), "starting KNX bridge");
    let mut child = Command::new(&bridge_command.executable)
        .args(&bridge_command.args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|source| HostError::Start {
            path: bridge_command.executable.clone(),
            source,
        })?;
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

    let result = run_session(&config, &mut engine, &mut child, &mut stdin, &mut reader).await;
    if result.is_err() {
        // A protocol/fatal/EOF error must not leave a sidecar behind.
        let _ = send_message(&mut stdin, &shutdown_message()).await;
        terminate_child(&mut child).await;
    }
    result
}

async fn forward_bridge_stderr<R>(stderr: R)
where
    R: AsyncRead + Unpin,
{
    let mut lines = BufReader::new(stderr).lines();
    loop {
        match lines.next_line().await {
            Ok(Some(line)) => tracing::debug!(target: "bridge.xknx", "{line}"),
            Ok(None) => break,
            Err(error) => {
                tracing::warn!(target: "bridge.xknx", error = %error, "failed reading bridge stderr");
                break;
            }
        }
    }
}

async fn run_session(
    config: &RuntimeConfig,
    engine: &mut Engine,
    child: &mut Child,
    stdin: &mut ChildStdin,
    reader: &mut BufReader<ChildStdout>,
) -> Result<(), HostError> {
    let hello = read_message(reader).await?;
    let hello = match hello {
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
    tracing::info!(
        target: "logiksmith",
        protocol = hello.v,
        bridge = %hello.bridge,
        bridge_version = %hello.bridge_version,
        xknx_version = %hello.xknx_version,
        "bridge hello"
    );

    let configure = configure_message(config);
    send_message(stdin, &configure).await?;
    tracing::info!(
        target: "logiksmith",
        gateway = %config.connection.gateway_ip,
        transport = "tunneling",
        "connecting KNX"
    );
    let ready = read_message(reader).await?;
    let ready = match ready {
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
    tracing::info!(target: "logiksmith", gateway = %ready.gateway, "KNX connected");
    tracing::info!(target: "logiksmith", "LogikSmith ready");

    let origin = Instant::now();
    let mut poll = time::interval(Duration::from_millis(20));
    poll.set_missed_tick_behavior(MissedTickBehavior::Skip);
    poll.tick().await;
    let mut line = String::new();
    let mut next_request_id = 1u64;
    let mut pending = HashSet::new();

    loop {
        tokio::select! {
            read = reader.read_line(&mut line) => {
                let bytes = read?;
                if bytes == 0 {
                    let status = child.wait().await?;
                    return Err(HostError::BridgeExited { status: format_status(status) });
                }
                let message = parse_message(line.trim_end_matches(['\r', '\n']))?;
                line.clear();
                match message {
                    Message::KnxEvent(event) => {
                        tracing::debug!(target: "logiksmith", source = ?event.source, destination = %event.destination, service = %event.service, dpt = %event.dpt, value = ?event.value, "KNX telegram received");
                        let event = event.to_core()?;
                        let commands = engine.handle_event(event, monotonic_now(origin));
                        dispatch_commands(stdin, commands, &mut next_request_id, &mut pending).await?;
                    }
                    Message::CommandResult(result) => {
                        if !pending.remove(&result.request_id) {
                            return Err(HostError::UnknownRequest(result.request_id));
                        }
                        if result.ok {
                            tracing::debug!(target: "logiksmith", request_id = result.request_id, "KNX write completed");
                        } else {
                            tracing::error!(target: "logiksmith", request_id = result.request_id, error = ?result.error, "KNX write failed");
                        }
                    }
                    Message::Fatal(fatal) => {
                        tracing::error!(target: "logiksmith", code = %fatal.code, message = %fatal.message, "bridge fatal");
                        return Err(HostError::BridgeFatal { code: fatal.code, message: fatal.message });
                    }
                    _ => return Err(ProtocolError::Field("runtime", "unexpected bridge message".to_owned()).into()),
                }
            }
            _ = poll.tick() => {
                let commands = engine.poll(monotonic_now(origin));
                dispatch_commands(stdin, commands, &mut next_request_id, &mut pending).await?;
            }
            signal = signal::ctrl_c() => {
                signal?;
                tracing::info!(target: "logiksmith", "shutting down");
                send_message(stdin, &shutdown_message()).await?;
                if time::timeout(Duration::from_secs(2), child.wait()).await.is_err() {
                    tracing::warn!(target: "logiksmith", "bridge did not stop after shutdown; terminating");
                    let _ = child.kill().await;
                    let _ = child.wait().await;
                }
                return Ok(());
            }
        }
    }
}

fn monotonic_now(origin: Instant) -> MonotonicMs {
    MonotonicMs(u64::try_from(origin.elapsed().as_millis()).unwrap_or(u64::MAX))
}

fn configure_message(config: &RuntimeConfig) -> Message {
    Message::Configure(Configure {
        v: PROTOCOL_VERSION,
        message_type: "configure".to_owned(),
        connection: TunnelingConnection {
            connection_type: "tunneling".to_owned(),
            gateway_ip: config.connection.gateway_ip.to_string(),
            gateway_port: config.connection.gateway_port,
            local_ip: config.connection.local_ip.map(|ip| ip.to_string()),
        },
        group_addresses: vec![
            GroupAddressDpt {
                address: config.engine.input_group_address.to_string(),
                dpt: DptMessage::bool(),
            },
            GroupAddressDpt {
                address: config.engine.output_group_address.to_string(),
                dpt: DptMessage::bool(),
            },
        ],
    })
}

fn shutdown_message() -> Message {
    Message::Shutdown(Shutdown {
        v: PROTOCOL_VERSION,
        message_type: "shutdown".to_owned(),
    })
}

async fn dispatch_commands(
    stdin: &mut ChildStdin,
    commands: Vec<CoreCommand>,
    next_request_id: &mut u64,
    pending: &mut HashSet<u64>,
) -> Result<(), HostError> {
    for command in commands {
        let CoreCommand::KnxWrite {
            destination,
            dpt,
            value,
        } = command;
        let request_id = *next_request_id;
        *next_request_id = next_request_id.checked_add(1).ok_or_else(|| {
            HostError::Protocol(ProtocolError::Field("request_id", "exhausted".to_owned()))
        })?;
        pending.insert(request_id);
        let value = match value {
            Value::Bool(value) => BoolValueMessage {
                kind: "bool".to_owned(),
                value,
            },
        };
        tracing::info!(target: "logiksmith", request_id, destination = %destination, dpt = %dpt, value = ?value, "KNX write requested");
        let message = Message::KnxWrite(KnxWrite {
            v: PROTOCOL_VERSION,
            message_type: "knx_write".to_owned(),
            request_id,
            destination: destination.to_string(),
            dpt: DptMessage {
                major: dpt.major,
                subtype: dpt.subtype,
            },
            value,
        });
        if let Err(error) = send_message(stdin, &message).await {
            pending.remove(&request_id);
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
    let line = encode_message(message)?;
    stdin.write_all(line.as_bytes()).await?;
    stdin.write_all(b"\n").await?;
    stdin.flush().await?;
    Ok(())
}

fn init_logging(config: LoggingConfig) {
    let filter = EnvFilter::builder()
        .with_default_directive(config.level.into())
        .parse_lossy(format!("bridge.xknx={}", config.bridge_level));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .try_init();
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value as JsonValue;

    fn round_trip(line: &str) {
        let parsed = parse_message(line).unwrap();
        let encoded = encode_message(&parsed).unwrap();
        assert_eq!(
            serde_json::from_str::<JsonValue>(line).unwrap(),
            serde_json::from_str::<JsonValue>(&encoded).unwrap()
        );
    }

    #[test]
    fn protocol_messages_round_trip() {
        round_trip(
            r#"{"v":1,"type":"bridge_hello","bridge":"xknx","bridge_version":"0.1.0","xknx_version":"3.0"}"#,
        );
        round_trip(
            r#"{"v":1,"type":"configure","connection":{"type":"tunneling","gateway_ip":"192.0.2.20","gateway_port":3671,"local_ip":null},"group_addresses":[{"address":"2/2/52","dpt":{"major":1,"subtype":1}},{"address":"2/3/52","dpt":{"major":1,"subtype":1}}]}"#,
        );
        round_trip(
            r#"{"v":1,"type":"ready","transport":"knxip_tunneling","gateway":"192.0.2.20"}"#,
        );
        round_trip(
            r#"{"v":1,"type":"knx_event","source":"1.1.42","destination":"2/2/52","service":"group_value_write","dpt":{"major":1,"subtype":1},"value":{"kind":"bool","value":true}}"#,
        );
        round_trip(
            r#"{"v":1,"type":"knx_write","request_id":12,"destination":"2/3/52","dpt":{"major":1,"subtype":1},"value":{"kind":"bool","value":true}}"#,
        );
        round_trip(r#"{"v":1,"type":"command_result","request_id":12,"ok":true}"#);
        round_trip(
            r#"{"v":1,"type":"fatal","code":"knx_connection_failed","message":"Unable to establish KNX/IP tunnel"}"#,
        );
        round_trip(r#"{"v":1,"type":"shutdown"}"#);
    }

    #[test]
    fn protocol_rejects_corruption_and_mismatch() {
        assert!(matches!(
            parse_message("not json"),
            Err(ProtocolError::Json(_))
        ));
        assert!(matches!(
            parse_message(
                r#"{"v":2,"type":"ready","transport":"knxip_tunneling","gateway":"192.0.2.20"}"#
            ),
            Err(ProtocolError::Version(2))
        ));
        assert!(matches!(
            parse_message(r#"{"v":1,"type":"unknown"}"#),
            Err(ProtocolError::MessageType(_))
        ));
        assert!(parse_message(r#"{"v":1,"type":"knx_event","source":"1.1.42","destination":"32/2/52","service":"group_value_write","dpt":{"major":1,"subtype":1},"value":{"kind":"bool","value":true}}"#).is_err());
        assert!(
            parse_message(r#"{"v":1,"type":"command_result","request_id":12,"ok":false}"#).is_err()
        );
    }
}
