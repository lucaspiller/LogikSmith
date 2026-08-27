//! Platform-independent KNX event processing for LogikSmith.
//!
//! The core has no I/O or clock access. Hosts provide events and monotonic
//! timestamps, then execute the commands returned by [`Engine`].

use std::{error::Error, fmt, str::FromStr};

/// A validated three-level KNX group address (`main/middle/subgroup`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GroupAddress {
    main: u8,
    middle: u8,
    subgroup: u8,
}

impl GroupAddress {
    /// Creates an address with KNX three-level ranges: 0..=31, 0..=7, and
    /// 0..=255 respectively. The all-zero address is reserved for broadcast.
    pub fn new(main: u16, middle: u16, subgroup: u16) -> Result<Self, GroupAddressError> {
        if main > 31 {
            return Err(GroupAddressError::ComponentOutOfRange {
                component: "main",
                value: main,
                max: 31,
            });
        }
        if middle > 7 {
            return Err(GroupAddressError::ComponentOutOfRange {
                component: "middle",
                value: middle,
                max: 7,
            });
        }
        if subgroup > 255 {
            return Err(GroupAddressError::ComponentOutOfRange {
                component: "subgroup",
                value: subgroup,
                max: 255,
            });
        }
        if main == 0 && middle == 0 && subgroup == 0 {
            return Err(GroupAddressError::BroadcastReserved);
        }

        Ok(Self {
            main: main as u8,
            middle: middle as u8,
            subgroup: subgroup as u8,
        })
    }

    /// Parses the canonical `main/middle/subgroup` representation.
    pub fn parse(value: &str) -> Result<Self, GroupAddressError> {
        value.parse()
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
}

impl FromStr for GroupAddress {
    type Err = GroupAddressError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let mut components = value.split('/');
        let main = components
            .next()
            .ok_or(GroupAddressError::InvalidFormat)?
            .parse::<u16>()
            .map_err(|_| GroupAddressError::InvalidFormat)?;
        let middle = components
            .next()
            .ok_or(GroupAddressError::InvalidFormat)?
            .parse::<u16>()
            .map_err(|_| GroupAddressError::InvalidFormat)?;
        let subgroup = components
            .next()
            .ok_or(GroupAddressError::InvalidFormat)?
            .parse::<u16>()
            .map_err(|_| GroupAddressError::InvalidFormat)?;
        if components.next().is_some() {
            return Err(GroupAddressError::InvalidFormat);
        }
        Self::new(main, middle, subgroup)
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
    BroadcastReserved,
    ComponentOutOfRange {
        component: &'static str,
        value: u16,
        max: u16,
    },
}

impl fmt::Display for GroupAddressError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFormat => write!(formatter, "group address must be main/middle/subgroup"),
            Self::BroadcastReserved => {
                write!(formatter, "group address 0/0/0 is reserved for broadcast")
            }
            Self::ComponentOutOfRange {
                component,
                value,
                max,
            } => write!(
                formatter,
                "group address {component} component {value} exceeds {max}"
            ),
        }
    }
}

impl Error for GroupAddressError {}

/// A validated three-level KNX individual (physical) address (`area.line.device`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct IndividualAddress {
    area: u8,
    line: u8,
    device: u8,
}

impl IndividualAddress {
    /// Creates an address with KNX ranges: 0..=15, 0..=15, and 0..=255.
    pub fn new(area: u16, line: u16, device: u16) -> Result<Self, IndividualAddressError> {
        if area > 15 {
            return Err(IndividualAddressError::ComponentOutOfRange {
                component: "area",
                value: area,
                max: 15,
            });
        }
        if line > 15 {
            return Err(IndividualAddressError::ComponentOutOfRange {
                component: "line",
                value: line,
                max: 15,
            });
        }
        if device > 255 {
            return Err(IndividualAddressError::ComponentOutOfRange {
                component: "device",
                value: device,
                max: 255,
            });
        }

        Ok(Self {
            area: area as u8,
            line: line as u8,
            device: device as u8,
        })
    }

    /// Parses the canonical `area.line.device` representation.
    pub fn parse(value: &str) -> Result<Self, IndividualAddressError> {
        value.parse()
    }

    pub const fn area(self) -> u8 {
        self.area
    }

    pub const fn line(self) -> u8 {
        self.line
    }

    pub const fn device(self) -> u8 {
        self.device
    }
}

impl FromStr for IndividualAddress {
    type Err = IndividualAddressError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let mut components = value.split('.');
        let area = components
            .next()
            .ok_or(IndividualAddressError::InvalidFormat)?
            .parse::<u16>()
            .map_err(|_| IndividualAddressError::InvalidFormat)?;
        let line = components
            .next()
            .ok_or(IndividualAddressError::InvalidFormat)?
            .parse::<u16>()
            .map_err(|_| IndividualAddressError::InvalidFormat)?;
        let device = components
            .next()
            .ok_or(IndividualAddressError::InvalidFormat)?
            .parse::<u16>()
            .map_err(|_| IndividualAddressError::InvalidFormat)?;
        if components.next().is_some() {
            return Err(IndividualAddressError::InvalidFormat);
        }
        Self::new(area, line, device)
    }
}

impl fmt::Display for IndividualAddress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}.{}", self.area, self.line, self.device)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IndividualAddressError {
    InvalidFormat,
    ComponentOutOfRange {
        component: &'static str,
        value: u16,
        max: u16,
    },
}

impl fmt::Display for IndividualAddressError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFormat => write!(formatter, "individual address must be area.line.device"),
            Self::ComponentOutOfRange {
                component,
                value,
                max,
            } => write!(
                formatter,
                "individual address {component} component {value} exceeds {max}"
            ),
        }
    }
}

impl Error for IndividualAddressError {}

/// A structured KNX datapoint type identifier.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Dpt {
    pub major: u16,
    pub subtype: u16,
}

impl Dpt {
    /// The only datapoint type supported by this proof of concept.
    pub const BOOL: Self = Self {
        major: 1,
        subtype: 1,
    };

    /// Creates a DPT identifier. KNX textual subtypes are three decimal digits.
    pub fn new(major: u16, subtype: u16) -> Result<Self, DptError> {
        if major == 0 {
            return Err(DptError::MajorOutOfRange(major));
        }
        if subtype > 999 {
            return Err(DptError::SubtypeOutOfRange(subtype));
        }
        Ok(Self { major, subtype })
    }

    /// Parses the canonical `major.subtype` representation, where subtype has
    /// exactly three decimal digits (for example, `1.001`).
    pub fn parse(value: &str) -> Result<Self, DptError> {
        value.parse()
    }

    pub const fn is_bool(self) -> bool {
        self.major == Self::BOOL.major && self.subtype == Self::BOOL.subtype
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
            Self::InvalidFormat => write!(
                formatter,
                "DPT must be formatted as major.subtype with a three-digit subtype"
            ),
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

/// A typed value carried by a KNX group telegram.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Value {
    Bool(bool),
}

/// KNX group services relevant to this proof of concept.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GroupService {
    Write,
    Response,
    Read,
}

/// An incoming KNX event supplied by a host or adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KnxEvent {
    pub source: Option<IndividualAddress>,
    pub destination: GroupAddress,
    pub service: GroupService,
    pub dpt: Dpt,
    pub value: Option<Value>,
}

/// A command for the host/transport adapter to execute.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Command {
    KnxWrite {
        destination: GroupAddress,
        dpt: Dpt,
        value: Value,
    },
}

/// A host-provided monotonic timestamp in milliseconds.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct MonotonicMs(pub u64);

impl MonotonicMs {
    pub const fn saturating_add(self, milliseconds: u64) -> Self {
        Self(self.0.saturating_add(milliseconds))
    }
}

/// Maximum accepted delay for this POC (24 hours).
pub const MAX_OFF_DELAY_MS: u64 = 86_400_000;

/// Configuration for the hard-coded POC behavior.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EngineConfig {
    pub input_group_address: GroupAddress,
    pub input_dpt: Dpt,
    pub output_group_address: GroupAddress,
    pub output_dpt: Dpt,
    pub off_delay_ms: u64,
}

impl EngineConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.input_group_address == self.output_group_address {
            return Err(ConfigError::SameGroupAddress);
        }
        if self.input_dpt != Dpt::BOOL {
            return Err(ConfigError::UnsupportedInputDpt(self.input_dpt));
        }
        if self.output_dpt != Dpt::BOOL {
            return Err(ConfigError::UnsupportedOutputDpt(self.output_dpt));
        }
        if self.off_delay_ms == 0 {
            return Err(ConfigError::ZeroOffDelay);
        }
        if self.off_delay_ms > MAX_OFF_DELAY_MS {
            return Err(ConfigError::OffDelayTooLarge {
                actual: self.off_delay_ms,
                maximum: MAX_OFF_DELAY_MS,
            });
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigError {
    SameGroupAddress,
    UnsupportedInputDpt(Dpt),
    UnsupportedOutputDpt(Dpt),
    ZeroOffDelay,
    OffDelayTooLarge { actual: u64, maximum: u64 },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SameGroupAddress => write!(
                formatter,
                "input_group_address and output_group_address must differ"
            ),
            Self::UnsupportedInputDpt(dpt) => {
                write!(formatter, "input_dpt must be 1.001, got {dpt}")
            }
            Self::UnsupportedOutputDpt(dpt) => {
                write!(formatter, "output_dpt must be 1.001, got {dpt}")
            }
            Self::ZeroOffDelay => write!(formatter, "off_delay_ms must be greater than zero"),
            Self::OffDelayTooLarge { actual, maximum } => {
                write!(formatter, "off_delay_ms {actual} exceeds maximum {maximum}")
            }
        }
    }
}

impl Error for ConfigError {}

/// Deterministic POC event-to-command engine.
#[derive(Clone, Copy, Debug)]
pub struct Engine {
    config: EngineConfig,
    off_deadline: Option<MonotonicMs>,
}

impl Engine {
    /// Constructs an engine, panicking if the configuration is invalid.
    /// Prefer [`Self::try_new`] at an external configuration boundary.
    pub fn new(config: EngineConfig) -> Self {
        Self::try_new(config).expect("invalid LogikSmith core configuration")
    }

    pub fn try_new(config: EngineConfig) -> Result<Self, ConfigError> {
        config.validate()?;
        Ok(Self {
            config,
            off_deadline: None,
        })
    }

    pub fn handle_event(&mut self, event: KnxEvent, now: MonotonicMs) -> Vec<Command> {
        if event.destination != self.config.input_group_address
            || event.service != GroupService::Write
            || event.dpt != self.config.input_dpt
            || event.value != Some(Value::Bool(true))
        {
            return Vec::new();
        }

        self.off_deadline = Some(now.saturating_add(self.config.off_delay_ms));
        vec![Command::KnxWrite {
            destination: self.config.output_group_address,
            dpt: self.config.output_dpt,
            value: Value::Bool(true),
        }]
    }

    pub fn poll(&mut self, now: MonotonicMs) -> Vec<Command> {
        if self.off_deadline.is_some_and(|deadline| now >= deadline) {
            self.off_deadline = None;
            return vec![Command::KnxWrite {
                destination: self.config.output_group_address,
                dpt: self.config.output_dpt,
                value: Value::Bool(false),
            }];
        }
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn address(value: &str) -> GroupAddress {
        value.parse().unwrap()
    }

    fn event(destination: &str, service: GroupService, dpt: Dpt, value: Option<Value>) -> KnxEvent {
        KnxEvent {
            source: None,
            destination: address(destination),
            service,
            dpt,
            value,
        }
    }

    fn config() -> EngineConfig {
        EngineConfig {
            input_group_address: address("2/2/52"),
            input_dpt: Dpt::BOOL,
            output_group_address: address("2/3/52"),
            output_dpt: Dpt::BOOL,
            off_delay_ms: 5_000,
        }
    }

    fn on_command() -> Command {
        Command::KnxWrite {
            destination: address("2/3/52"),
            dpt: Dpt::BOOL,
            value: Value::Bool(true),
        }
    }

    fn off_command() -> Command {
        Command::KnxWrite {
            destination: address("2/3/52"),
            dpt: Dpt::BOOL,
            value: Value::Bool(false),
        }
    }

    #[test]
    fn trigger_produces_on() {
        let mut engine = Engine::new(config());
        assert_eq!(
            engine.handle_event(
                event(
                    "2/2/52",
                    GroupService::Write,
                    Dpt::BOOL,
                    Some(Value::Bool(true))
                ),
                MonotonicMs(1_000)
            ),
            vec![on_command()]
        );
    }

    #[test]
    fn timer_does_not_fire_early() {
        let mut engine = Engine::new(config());
        engine.handle_event(
            event(
                "2/2/52",
                GroupService::Write,
                Dpt::BOOL,
                Some(Value::Bool(true)),
            ),
            MonotonicMs(1_000),
        );
        assert!(engine.poll(MonotonicMs(5_999)).is_empty());
    }

    #[test]
    fn timer_fires_at_deadline() {
        let mut engine = Engine::new(config());
        engine.handle_event(
            event(
                "2/2/52",
                GroupService::Write,
                Dpt::BOOL,
                Some(Value::Bool(true)),
            ),
            MonotonicMs(1_000),
        );
        assert_eq!(engine.poll(MonotonicMs(6_000)), vec![off_command()]);
        assert!(engine.poll(MonotonicMs(6_001)).is_empty());
    }

    #[test]
    fn false_does_not_trigger() {
        let mut engine = Engine::new(config());
        assert!(
            engine
                .handle_event(
                    event(
                        "2/2/52",
                        GroupService::Write,
                        Dpt::BOOL,
                        Some(Value::Bool(false))
                    ),
                    MonotonicMs(1_000)
                )
                .is_empty()
        );
        assert!(engine.poll(MonotonicMs(6_000)).is_empty());
    }

    #[test]
    fn wrong_address_does_not_trigger() {
        let mut engine = Engine::new(config());
        assert!(
            engine
                .handle_event(
                    event(
                        "2/2/53",
                        GroupService::Write,
                        Dpt::BOOL,
                        Some(Value::Bool(true))
                    ),
                    MonotonicMs(1_000)
                )
                .is_empty()
        );
    }

    #[test]
    fn group_value_response_does_not_trigger() {
        let mut engine = Engine::new(config());
        assert!(
            engine
                .handle_event(
                    event(
                        "2/2/52",
                        GroupService::Response,
                        Dpt::BOOL,
                        Some(Value::Bool(true))
                    ),
                    MonotonicMs(1_000)
                )
                .is_empty()
        );
    }

    #[test]
    fn retrigger_resets_deadline() {
        let mut engine = Engine::new(config());
        assert_eq!(
            engine.handle_event(
                event(
                    "2/2/52",
                    GroupService::Write,
                    Dpt::BOOL,
                    Some(Value::Bool(true))
                ),
                MonotonicMs(1_000)
            ),
            vec![on_command()]
        );
        assert_eq!(
            engine.handle_event(
                event(
                    "2/2/52",
                    GroupService::Write,
                    Dpt::BOOL,
                    Some(Value::Bool(true))
                ),
                MonotonicMs(4_000)
            ),
            vec![on_command()]
        );
        assert!(engine.poll(MonotonicMs(6_000)).is_empty());
        assert_eq!(engine.poll(MonotonicMs(9_000)), vec![off_command()]);
    }

    #[test]
    fn old_timer_cannot_switch_light_off() {
        let mut engine = Engine::new(config());
        engine.handle_event(
            event(
                "2/2/52",
                GroupService::Write,
                Dpt::BOOL,
                Some(Value::Bool(true)),
            ),
            MonotonicMs(1_000),
        );
        engine.handle_event(
            event(
                "2/2/52",
                GroupService::Write,
                Dpt::BOOL,
                Some(Value::Bool(true)),
            ),
            MonotonicMs(4_000),
        );
        assert!(engine.poll(MonotonicMs(5_999)).is_empty());
        assert_eq!(engine.poll(MonotonicMs(6_000)), Vec::<Command>::new());
    }

    #[test]
    fn unsupported_value_and_dpt_do_not_trigger() {
        let mut engine = Engine::new(config());
        assert!(
            engine
                .handle_event(
                    event(
                        "2/2/52",
                        GroupService::Write,
                        Dpt {
                            major: 5,
                            subtype: 1
                        },
                        Some(Value::Bool(true))
                    ),
                    MonotonicMs(1_000)
                )
                .is_empty()
        );
        assert!(
            engine
                .handle_event(
                    event("2/2/52", GroupService::Write, Dpt::BOOL, None),
                    MonotonicMs(1_000)
                )
                .is_empty()
        );
    }

    #[test]
    fn read_does_not_trigger() {
        let mut engine = Engine::new(config());
        assert!(
            engine
                .handle_event(
                    event("2/2/52", GroupService::Read, Dpt::BOOL, None),
                    MonotonicMs(1_000)
                )
                .is_empty()
        );
    }

    #[test]
    fn addresses_parse_and_format() {
        assert_eq!(address("2/3/52").to_string(), "2/3/52");
        assert_eq!(address("31/7/255").to_string(), "31/7/255");
        assert!("32/0/0".parse::<GroupAddress>().is_err());
        assert!("0/8/0".parse::<GroupAddress>().is_err());
        assert!("0/0/0".parse::<GroupAddress>().is_err());
        assert!("2/3".parse::<GroupAddress>().is_err());
        assert!("2/3/x".parse::<GroupAddress>().is_err());
        assert_eq!(
            "1.2.3".parse::<IndividualAddress>().unwrap().to_string(),
            "1.2.3"
        );
        assert!("16.0.0".parse::<IndividualAddress>().is_err());
        assert!("1.2".parse::<IndividualAddress>().is_err());
    }

    #[test]
    fn dpt_parses_and_formats() {
        assert_eq!("1.001".parse::<Dpt>().unwrap(), Dpt::BOOL);
        assert_eq!("232.600".parse::<Dpt>().unwrap().to_string(), "232.600");
        assert!("1.1".parse::<Dpt>().is_err());
        assert!("0.001".parse::<Dpt>().is_err());
        assert!("1.1000".parse::<Dpt>().is_err());
    }

    #[test]
    fn configuration_boundaries_are_validated() {
        let mut invalid = config();
        invalid.input_group_address = invalid.output_group_address;
        assert_eq!(invalid.validate(), Err(ConfigError::SameGroupAddress));

        let mut invalid = config();
        invalid.input_dpt = Dpt {
            major: 5,
            subtype: 1,
        };
        assert_eq!(
            invalid.validate(),
            Err(ConfigError::UnsupportedInputDpt(invalid.input_dpt))
        );

        let mut invalid = config();
        invalid.output_dpt = Dpt {
            major: 5,
            subtype: 1,
        };
        assert_eq!(
            invalid.validate(),
            Err(ConfigError::UnsupportedOutputDpt(invalid.output_dpt))
        );

        let mut invalid = config();
        invalid.off_delay_ms = 0;
        assert_eq!(invalid.validate(), Err(ConfigError::ZeroOffDelay));

        let mut invalid = config();
        invalid.off_delay_ms = MAX_OFF_DELAY_MS + 1;
        assert_eq!(
            invalid.validate(),
            Err(ConfigError::OffDelayTooLarge {
                actual: MAX_OFF_DELAY_MS + 1,
                maximum: MAX_OFF_DELAY_MS
            })
        );
    }
}
