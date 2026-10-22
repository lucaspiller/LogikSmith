use logiksmith_core::{
    BlockConfig, BlockId, BlockSchedule, Dpt, Endpoint, EndpointDirection, EndpointName,
    EngineConfig, RuntimeConfig as CoreRuntimeConfig, RuntimeLimits, RuntimeProfile, ScheduleName,
    ScheduleRule, SignalBinding as CoreSignalBinding, SignalConfig, SignalName, SolarAnchor,
    Weekday, WeekdaySet,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fmt,
    str::FromStr,
    time::Duration,
};

#[path = "document_build.rs"]
mod document_build;
pub use document_build::*;

pub const MAX_BLOCKS: usize = 64;
pub const MAX_SIGNALS: usize = 256;
pub const MAX_SIGNAL_BINDINGS: usize = 256;
pub const MAX_SCHEDULES_PER_BLOCK: usize = 32;
pub const MAX_EXTERNAL_SOURCES: usize = 64;
pub const MAX_EXTERNAL_VALUES_AND_BINDINGS: usize = 256;
pub const MAX_LOGIC_SOURCE_BYTES: usize = logiksmith_core::MAX_LOGIC_SOURCE_BYTES;
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct Capabilities {
    pub timezones: bool,
    pub astronomy: bool,
    pub http_inputs: bool,
    pub webhook_inputs: bool,
    pub schedules: bool,
    pub persistent_state: bool,
}
impl Default for Capabilities {
    fn default() -> Self {
        Self::desktop()
    }
}
impl Capabilities {
    pub const fn desktop() -> Self {
        Self {
            timezones: cfg!(feature = "timezones"),
            astronomy: cfg!(feature = "astronomy"),
            http_inputs: cfg!(feature = "http-inputs"),
            webhook_inputs: cfg!(feature = "webhook-inputs"),
            schedules: true,
            persistent_state: true,
        }
    }
    pub const fn embedded() -> Self {
        Self {
            timezones: cfg!(feature = "timezones"),
            astronomy: cfg!(feature = "astronomy"),
            http_inputs: false,
            webhook_inputs: false,
            schedules: false,
            persistent_state: false,
        }
    }
}
pub const fn compiled_capabilities() -> Capabilities {
    Capabilities::desktop()
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
    pub const fn new(main: u8, middle: u8, subgroup: u8) -> Result<Self, GroupAddressError> {
        if main > 31 || middle > 7 {
            return Err(GroupAddressError::OutOfRange);
        }
        if main == 0 && middle == 0 && subgroup == 0 {
            return Err(GroupAddressError::BroadcastReserved);
        }
        Ok(Self {
            main,
            middle,
            subgroup,
        })
    }
    pub const fn main(self) -> u8 {
        self.main
    }
    pub const fn middle(self) -> u8 {
        self.middle
    }
    pub const fn subgroup(self) -> u8 {
        self.subgroup
    }
    pub fn as_u16(self) -> u16 {
        (u16::from(self.main) << 11) | (u16::from(self.middle) << 8) | u16::from(self.subgroup)
    }
}
impl FromStr for GroupAddress {
    type Err = GroupAddressError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let parts: Vec<_> = value.split('/').collect();
        if parts.len() != 3
            || parts
                .iter()
                .any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()))
        {
            return Err(GroupAddressError::InvalidFormat);
        }
        if parts.iter().any(|p| p.len() > 1 && p.starts_with('0')) {
            return Err(GroupAddressError::NonCanonical);
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
        Self::new(main as u8, middle as u8, subgroup as u8)
    }
}
impl fmt::Display for GroupAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}/{}", self.main, self.middle, self.subgroup)
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
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidFormat => "group address must be main/middle/subgroup",
            Self::OutOfRange => "group address component is out of range",
            Self::BroadcastReserved => "group address 0/0/0 is reserved for broadcast",
            Self::NonCanonical => "group address must use canonical main/middle/subgroup form",
        })
    }
}
impl std::error::Error for GroupAddressError {}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AutomationDocument {
    #[serde(default)]
    pub signals: Vec<AutomationSignal>,
    #[serde(default)]
    pub http_polls: Vec<HttpPoll>,
    #[serde(default)]
    pub webhook_inputs: Vec<WebhookInput>,
    pub blocks: Vec<AutomationBlock>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AutomationSignal {
    pub name: String,
    pub dpt: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
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
    #[serde(default)]
    pub http_bindings: Vec<HttpBinding>,
    #[serde(default)]
    pub webhook_bindings: Vec<WebhookBinding>,
    pub source: String,
    #[serde(default)]
    pub schedules: Vec<AutomationSchedule>,
}
fn default_block_revision() -> u64 {
    1
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
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
    pub extra: BTreeMap<String, serde_json::Value>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AutomationEndpoint {
    pub name: String,
    pub dpt: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct KnxBinding {
    pub endpoint: String,
    pub group_address: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SignalBinding {
    pub endpoint: String,
    pub signal: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HttpPoll {
    pub name: String,
    pub url: String,
    pub every: String,
    pub timeout: String,
    pub stale_after: String,
    #[serde(default)]
    pub headers: Vec<HttpHeader>,
    pub values: Vec<HttpPollValue>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HttpHeader {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_env: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HttpPollValue {
    pub name: String,
    pub dpt: String,
    pub json_pointer: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WebhookInput {
    pub name: String,
    pub dpt: String,
    pub json_pointer: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bearer_token_env: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HttpBinding {
    pub endpoint: String,
    pub source: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WebhookBinding {
    pub endpoint: String,
    pub source: String,
}
#[derive(Clone, Debug)]
pub struct HttpPollRuntime {
    pub name: String,
    pub url: String,
    pub every: std::time::Duration,
    pub timeout: std::time::Duration,
    pub stale_after: std::time::Duration,
    pub headers: Vec<(String, String)>,
    pub values: Vec<HttpPollValueRuntime>,
}
#[derive(Clone, Debug)]
pub struct HttpPollValueRuntime {
    pub name: String,
    pub dpt: Dpt,
    pub json_pointer: String,
}
#[derive(Clone, Debug)]
pub struct WebhookInputRuntime {
    pub name: String,
    pub dpt: Dpt,
    pub json_pointer: String,
    pub bearer_token: Option<String>,
}
#[derive(Clone, Debug)]
pub struct AutomationRuntime {
    pub document: AutomationDocument,
    pub signals: Vec<SignalRuntime>,
    pub core_config: CoreRuntimeConfig,
    pub blocks: Vec<BlockRuntime>,
    pub address_to_inputs: HashMap<GroupAddress, Vec<BlockInputBinding>>,
    pub output_to_address: HashMap<(BlockId, EndpointName), GroupAddress>,
    pub signal_to_inputs: HashMap<SignalName, Vec<BlockSignalInputBinding>>,
    pub output_to_signal: HashMap<(BlockId, EndpointName), SignalName>,
    pub http_to_inputs: HashMap<String, Vec<BlockExternalInputBinding>>,
    pub webhook_to_inputs: HashMap<String, Vec<BlockExternalInputBinding>>,
    pub http_polls: Vec<HttpPollRuntime>,
    pub webhook_inputs: Vec<WebhookInputRuntime>,
    pub address_dpts: HashMap<GroupAddress, Dpt>,
    pub structural_revision: u64,
    pub document_revision: u64,
}
#[derive(Clone, Debug)]
pub struct BlockRuntime {
    pub id: BlockId,
    pub revision: u64,
    pub enabled: bool,
    pub engine_config: EngineConfig,
    pub endpoint_to_address: HashMap<EndpointName, GroupAddress>,
    pub endpoint_to_signal: HashMap<EndpointName, SignalName>,
    pub endpoint_to_external: HashMap<EndpointName, String>,
    pub endpoint_dpts: HashMap<EndpointName, Dpt>,
    pub schedules: Vec<BlockSchedule>,
}
#[derive(Clone, Debug)]
pub struct SignalRuntime {
    pub name: SignalName,
    pub dpt: Dpt,
}
#[derive(Clone, Debug)]
pub struct BlockInputBinding {
    pub block_id: BlockId,
    pub endpoint: EndpointName,
    pub dpt: Dpt,
    pub address: GroupAddress,
}
#[derive(Clone, Debug)]
pub struct BlockSignalInputBinding {
    pub block_id: BlockId,
    pub endpoint: EndpointName,
    pub dpt: Dpt,
    pub signal: SignalName,
}
#[derive(Clone, Debug)]
pub struct BlockExternalInputBinding {
    pub block_id: BlockId,
    pub endpoint: EndpointName,
    pub dpt: Dpt,
    pub source: String,
}
impl AutomationRuntime {
    pub fn block(&self, id: &BlockId) -> Option<&BlockRuntime> {
        self.blocks.iter().find(|b| b.id == *id)
    }
    pub fn block_ids(&self) -> impl Iterator<Item = &BlockId> {
        self.blocks.iter().map(|b| &b.id)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct FieldError {
    pub path: String,
    pub message: String,
}
fn field(path: impl Into<String>, message: impl Into<String>) -> FieldError {
    FieldError {
        path: path.into(),
        message: message.into(),
    }
}
fn endpoint_name(path: &str, value: &str) -> Result<EndpointName, FieldError> {
    value
        .parse::<EndpointName>()
        .map_err(|e| field(path, e.to_string()))
}
fn parse_dpt(path: &str, value: &str) -> Result<Dpt, FieldError> {
    let dpt = Dpt::parse(value).map_err(|e| field(path, e.to_string()))?;
    if !dpt.is_supported() {
        return Err(field(path, "must be 1.001, 5.001, or 9.001"));
    }
    if dpt.to_string() != value {
        return Err(field(path, "must use canonical DPT form"));
    }
    Ok(dpt)
}
fn source_name(path: &str, value: &str) -> Result<String, FieldError> {
    let name = endpoint_name(path, value)?;
    if name.as_str() != value {
        return Err(field(path, "must use canonical identifier form"));
    }
    Ok(value.to_owned())
}
fn validate_pointer(path: &str, pointer: &str, errors: &mut Vec<FieldError>) {
    if pointer.is_empty() {
        return;
    }
    if !pointer.starts_with('/') {
        errors.push(field(
            path,
            "must be empty or an RFC 6901 pointer beginning with '/'",
        ));
        return;
    }
    for token in pointer.split('/').skip(1) {
        let bytes = token.as_bytes();
        if bytes.iter().enumerate().any(|(i, b)| {
            *b == b'~' && (i + 1 >= bytes.len() || !matches!(bytes[i + 1], b'0' | b'1'))
        }) {
            errors.push(field(path, "must use valid RFC 6901 '~0' and '~1' escapes"));
            return;
        }
    }
}
fn parse_duration(value: &str, signed: bool) -> Result<i64, ()> {
    let (negative, rest) = if signed && value.starts_with('-') {
        (true, &value[1..])
    } else {
        (false, value)
    };
    if rest.is_empty() {
        return Err(());
    }
    let mut total = 0i64;
    let mut digits = String::new();
    let mut rank = 4u8;
    for c in rest.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
            continue;
        }
        let amount = digits.parse::<i64>().map_err(|_| ())?;
        digits.clear();
        let (unit, current) = match c {
            'h' => (3600, 3),
            'm' => (60, 2),
            's' => (1, 1),
            _ => return Err(()),
        };
        if current >= rank {
            return Err(());
        }
        rank = current;
        total = total
            .checked_add(amount.checked_mul(unit).ok_or(())?)
            .ok_or(())?;
    }
    if digits.is_empty() && rank != 4 {
        Ok(if negative { -total } else { total })
    } else {
        Err(())
    }
}
fn parse_local_time(
    path: &str,
    value: &str,
    errors: &mut Vec<FieldError>,
) -> Option<logiksmith_core::LocalTime> {
    let p: Vec<_> = value.split(':').collect();
    if !matches!(p.len(), 2 | 3)
        || p.iter()
            .any(|v| v.len() != 2 || !v.bytes().all(|b| b.is_ascii_digit()))
    {
        errors.push(field(
            path,
            "must be a canonical local time HH:MM or HH:MM:SS",
        ));
        return None;
    }
    let h = p[0].parse::<u8>().ok();
    let m = p[1].parse::<u8>().ok();
    let s = p.get(2).map_or(Some(0), |v| v.parse().ok());
    if !matches!((h,m,s), (Some(h),Some(m),Some(s)) if h <= 23 && m <= 59 && s <= 59) {
        errors.push(field(path, "must be a valid local time"));
        return None;
    }
    Some(logiksmith_core::LocalTime {
        hour: h.unwrap(),
        minute: m.unwrap(),
        second: s.unwrap(),
    })
}
fn weekdays(path: &str, values: Option<&[String]>, errors: &mut Vec<FieldError>) -> WeekdaySet {
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    for (i, token) in values.unwrap_or(&[]).iter().enumerate() {
        let day = match token.as_str() {
            "mon" => Weekday::Monday,
            "tue" => Weekday::Tuesday,
            "wed" => Weekday::Wednesday,
            "thu" => Weekday::Thursday,
            "fri" => Weekday::Friday,
            "sat" => Weekday::Saturday,
            "sun" => Weekday::Sunday,
            _ => {
                errors.push(field(
                    format!("{path}[{i}]"),
                    "must be one of mon, tue, wed, thu, fri, sat, or sun",
                ));
                continue;
            }
        };
        if !seen.insert(day) {
            errors.push(field(format!("{path}[{i}]"), "must not repeat a weekday"));
        } else {
            result.push(day);
        }
    }
    if values.is_none() {
        result = Weekday::ALL.to_vec();
    }
    result.sort_by_key(|day| match day {
        Weekday::Monday => 0,
        Weekday::Tuesday => 1,
        Weekday::Wednesday => 2,
        Weekday::Thursday => 3,
        Weekday::Friday => 4,
        Weekday::Saturday => 5,
        Weekday::Sunday => 6,
    });
    WeekdaySet::new(&result).unwrap_or_else(|_| WeekdaySet::new(&Weekday::ALL).unwrap())
}
fn schedule_rule(
    schedule: &AutomationSchedule,
    path: &str,
    errors: &mut Vec<FieldError>,
) -> Option<ScheduleRule> {
    if !schedule.extra.is_empty() {
        for key in schedule.extra.keys() {
            errors.push(field(format!("{path}.{key}"), "unknown field for schedule"));
        }
    }
    match schedule.kind.as_str() {
        "fixed" => {
            let at = schedule
                .at
                .as_deref()
                .and_then(|v| parse_local_time(&format!("{path}.at"), v, errors));
            if schedule.at.is_none() {
                errors.push(field(
                    format!("{path}.at"),
                    "is required for a fixed schedule",
                ));
            }
            if schedule.every.is_some() || schedule.offset.is_some() || schedule.anchor.is_some() {
                errors.push(field(
                    path,
                    "contains fields not allowed for a fixed schedule",
                ));
            }
            Some(ScheduleRule::Fixed {
                at: at?,
                weekdays: weekdays(
                    &format!("{path}.weekdays"),
                    schedule.weekdays.as_deref(),
                    errors,
                ),
            })
        }
        "interval" => {
            let every = schedule
                .every
                .as_deref()
                .and_then(|v| parse_duration(v, false).ok());
            if every.is_none() {
                errors.push(field(
                    format!("{path}.every"),
                    "must be a whole-second duration and is required",
                ));
            }
            let offset = schedule
                .offset
                .as_deref()
                .map_or(Some(0), |v| parse_duration(v, false).ok());
            if schedule.offset.is_some() && offset.is_none() {
                errors.push(field(
                    format!("{path}.offset"),
                    "must be a whole-second duration",
                ));
            }
            let (Some(every), Some(offset)) = (every, offset) else {
                return None;
            };
            if !(60..=604800).contains(&every) {
                errors.push(field(
                    format!("{path}.every"),
                    "must be between 60s and 7 days (604800s)",
                ));
                return None;
            }
            if !(0..every).contains(&offset) {
                errors.push(field(
                    format!("{path}.offset"),
                    "must be between 0s and every - 1s",
                ));
                return None;
            }
            if schedule.at.is_some() || schedule.anchor.is_some() || schedule.weekdays.is_some() {
                errors.push(field(
                    path,
                    "contains fields not allowed for an interval schedule",
                ));
            }
            Some(ScheduleRule::Interval {
                every_seconds: every as u32,
                offset_seconds: offset as u32,
            })
        }
        "astronomical" => {
            let anchor = match schedule.anchor.as_deref() {
                Some("dawn") => Some(SolarAnchor::Dawn),
                Some("sunrise") => Some(SolarAnchor::Sunrise),
                Some("sunset") => Some(SolarAnchor::Sunset),
                Some("dusk") => Some(SolarAnchor::Dusk),
                Some(_) => {
                    errors.push(field(
                        format!("{path}.anchor"),
                        "must be one of dawn, sunrise, sunset, or dusk",
                    ));
                    None
                }
                None => {
                    errors.push(field(
                        format!("{path}.anchor"),
                        "is required for an astronomical schedule",
                    ));
                    None
                }
            };
            let offset = schedule
                .offset
                .as_deref()
                .and_then(|v| parse_duration(v, true).ok());
            if offset.is_none() {
                errors.push(field(
                    format!("{path}.offset"),
                    "must be a signed whole-second duration and is required",
                ));
            }
            let (Some(anchor), Some(offset)) = (anchor, offset) else {
                return None;
            };
            if !(-86400..=86400).contains(&offset) {
                errors.push(field(
                    format!("{path}.offset"),
                    "must be between -24h and +24h",
                ));
                return None;
            }
            if schedule.at.is_some() || schedule.every.is_some() {
                errors.push(field(
                    path,
                    "contains fields not allowed for an astronomical schedule",
                ));
            }
            Some(ScheduleRule::Astronomical {
                anchor,
                offset_seconds: offset as i32,
                weekdays: weekdays(
                    &format!("{path}.weekdays"),
                    schedule.weekdays.as_deref(),
                    errors,
                ),
            })
        }
        _ => {
            errors.push(field(
                format!("{path}.kind"),
                "must be one of fixed, interval, or astronomical",
            ));
            None
        }
    }
}
