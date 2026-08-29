use crate::*;
use logiksmith_core::TypedValue;
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, ffi::OsString, net::IpAddr, path::PathBuf};
use thiserror::Error;

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
    pub(crate) fn typed_value(&self) -> Result<Option<TypedValue>, ProtocolError> {
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
