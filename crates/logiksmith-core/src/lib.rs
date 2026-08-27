//! Platform-independent event processing for LogikSmith.
//!
//! The core deals in named, typed endpoints. Hosts provide input events and
//! monotonic timestamps, then execute the logical effects returned by
//! [`Engine`]. Transport details such as KNX group addresses stay outside this
//! crate.

use std::{error::Error, fmt, str::FromStr};

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

    /// Creates a DPT identifier. Supported endpoint declarations are checked
    /// separately by [`EngineConfig::validate`].
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

/// An input event supplied by a host or adapter.
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

/// A logical output effect for the host to execute.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Effect {
    SetOutput {
        endpoint: EndpointName,
        value: TypedValue,
    },
}

/// The selected timed boolean behaviour.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimedBooleanRule {
    pub input: EndpointName,
    pub output: EndpointName,
    pub off_delay_ms: u64,
}

/// Compatibility spelling for the rule name used by the TOML section.
pub type TimedBoolRule = TimedBooleanRule;

/// The selected percentage forwarding behaviour.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PercentageForwardRule {
    pub input: EndpointName,
    pub output: EndpointName,
}

/// Configuration for the portable endpoint engine.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EngineConfig {
    pub endpoints: Vec<Endpoint>,
    pub timed_bool: TimedBooleanRule,
    pub percentage_forward: PercentageForwardRule,
}

impl EngineConfig {
    pub fn new(
        endpoints: Vec<Endpoint>,
        timed_bool: TimedBooleanRule,
        percentage_forward: PercentageForwardRule,
    ) -> Self {
        Self {
            endpoints,
            timed_bool,
            percentage_forward,
        }
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

        validate_rule_endpoint(
            &self.endpoints,
            "timed_bool",
            "input",
            &self.timed_bool.input,
            EndpointDirection::Input,
            Dpt::BOOL,
        )?;
        validate_rule_endpoint(
            &self.endpoints,
            "timed_bool",
            "output",
            &self.timed_bool.output,
            EndpointDirection::Output,
            Dpt::BOOL,
        )?;
        if self.timed_bool.off_delay_ms == 0 {
            return Err(ConfigError::ZeroOffDelay);
        }
        if self.timed_bool.off_delay_ms > MAX_OFF_DELAY_MS {
            return Err(ConfigError::OffDelayTooLarge {
                actual: self.timed_bool.off_delay_ms,
                maximum: MAX_OFF_DELAY_MS,
            });
        }

        validate_rule_endpoint(
            &self.endpoints,
            "percentage_forward",
            "input",
            &self.percentage_forward.input,
            EndpointDirection::Input,
            Dpt::PERCENT,
        )?;
        validate_rule_endpoint(
            &self.endpoints,
            "percentage_forward",
            "output",
            &self.percentage_forward.output,
            EndpointDirection::Output,
            Dpt::PERCENT,
        )?;
        Ok(())
    }
}

fn validate_rule_endpoint(
    endpoints: &[Endpoint],
    rule: &'static str,
    role: &'static str,
    name: &EndpointName,
    expected_direction: EndpointDirection,
    expected_dpt: Dpt,
) -> Result<(), ConfigError> {
    let endpoint = endpoints
        .iter()
        .find(|endpoint| endpoint.name == *name)
        .ok_or_else(|| ConfigError::UnknownRuleEndpoint {
            rule,
            role,
            endpoint: name.clone(),
        })?;
    if endpoint.direction != expected_direction {
        return Err(ConfigError::WrongRuleDirection {
            rule,
            role,
            endpoint: name.clone(),
            expected: expected_direction,
            actual: endpoint.direction,
        });
    }
    if endpoint.dpt != expected_dpt {
        return Err(ConfigError::WrongRuleDpt {
            rule,
            role,
            endpoint: name.clone(),
            expected: expected_dpt,
            actual: endpoint.dpt,
        });
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConfigError {
    DuplicateEndpoint(EndpointName),
    UnsupportedEndpointDpt {
        endpoint: EndpointName,
        dpt: Dpt,
    },
    UnknownRuleEndpoint {
        rule: &'static str,
        role: &'static str,
        endpoint: EndpointName,
    },
    WrongRuleDirection {
        rule: &'static str,
        role: &'static str,
        endpoint: EndpointName,
        expected: EndpointDirection,
        actual: EndpointDirection,
    },
    WrongRuleDpt {
        rule: &'static str,
        role: &'static str,
        endpoint: EndpointName,
        expected: Dpt,
        actual: Dpt,
    },
    ZeroOffDelay,
    OffDelayTooLarge {
        actual: u64,
        maximum: u64,
    },
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
            Self::UnknownRuleEndpoint {
                rule,
                role,
                endpoint,
            } => write!(
                formatter,
                "{rule}.{role} references unknown endpoint {endpoint}"
            ),
            Self::WrongRuleDirection {
                rule,
                role,
                endpoint,
                expected,
                actual,
            } => write!(
                formatter,
                "{rule}.{role} endpoint {endpoint} must be {expected}, got {actual}"
            ),
            Self::WrongRuleDpt {
                rule,
                role,
                endpoint,
                expected,
                actual,
            } => write!(
                formatter,
                "{rule}.{role} endpoint {endpoint} must use DPT {expected}, got {actual}"
            ),
            Self::ZeroOffDelay => {
                formatter.write_str("timed_bool.off_delay_ms must be greater than zero")
            }
            Self::OffDelayTooLarge { actual, maximum } => write!(
                formatter,
                "timed_bool.off_delay_ms {actual} exceeds maximum {maximum}"
            ),
        }
    }
}

impl Error for ConfigError {}

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

/// A host-provided monotonic timestamp in milliseconds.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct MonotonicMs(pub u64);

impl MonotonicMs {
    pub const fn saturating_add(self, milliseconds: u64) -> Self {
        Self(self.0.saturating_add(milliseconds))
    }
}

/// Maximum accepted delay for the timed behaviour (24 hours).
pub const MAX_OFF_DELAY_MS: u64 = 86_400_000;

/// Read-only view of the engine's timer state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EngineSnapshot {
    pub off_deadline: Option<MonotonicMs>,
}

/// Deterministic endpoint event-to-effect engine.
#[derive(Clone, Debug)]
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

    pub fn snapshot(&self) -> EngineSnapshot {
        EngineSnapshot {
            off_deadline: self.off_deadline,
        }
    }

    pub fn handle_event(
        &mut self,
        event: InputEvent,
        now: MonotonicMs,
    ) -> Result<Vec<Effect>, EventError> {
        event.value.validate().map_err(EventError::InvalidValue)?;
        let endpoint = self
            .config
            .endpoints
            .iter()
            .find(|endpoint| endpoint.name == event.endpoint)
            .ok_or_else(|| EventError::UnknownEndpoint(event.endpoint.clone()))?;
        if endpoint.direction != EndpointDirection::Input {
            return Err(EventError::EndpointNotInput {
                endpoint: event.endpoint,
                actual: endpoint.direction,
            });
        }
        if endpoint.dpt != event.value.dpt {
            return Err(EventError::DptMismatch {
                endpoint: event.endpoint,
                expected: endpoint.dpt,
                actual: event.value.dpt,
            });
        }

        if event.endpoint == self.config.timed_bool.input && event.value.value == Value::Bool(true)
        {
            self.off_deadline = Some(now.saturating_add(self.config.timed_bool.off_delay_ms));
            return Ok(vec![Effect::SetOutput {
                endpoint: self.config.timed_bool.output.clone(),
                value: TypedValue::bool(true),
            }]);
        }

        if event.endpoint == self.config.percentage_forward.input {
            return Ok(vec![Effect::SetOutput {
                endpoint: self.config.percentage_forward.output.clone(),
                value: event.value,
            }]);
        }

        Ok(Vec::new())
    }

    pub fn poll(&mut self, now: MonotonicMs) -> Vec<Effect> {
        if self.off_deadline.is_some_and(|deadline| now >= deadline) {
            self.off_deadline = None;
            return vec![Effect::SetOutput {
                endpoint: self.config.timed_bool.output.clone(),
                value: TypedValue::bool(false),
            }];
        }
        Vec::new()
    }
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

    fn config() -> EngineConfig {
        EngineConfig::new(
            vec![
                endpoint("wall_switch", EndpointDirection::Input, Dpt::BOOL),
                endpoint("dimmer_level", EndpointDirection::Input, Dpt::PERCENT),
                endpoint("test_light", EndpointDirection::Output, Dpt::BOOL),
                endpoint("dimmer_output", EndpointDirection::Output, Dpt::PERCENT),
                endpoint("unused_input", EndpointDirection::Input, Dpt::BOOL),
            ],
            TimedBooleanRule {
                input: name("wall_switch"),
                output: name("test_light"),
                off_delay_ms: 5_000,
            },
            PercentageForwardRule {
                input: name("dimmer_level"),
                output: name("dimmer_output"),
            },
        )
    }

    fn timed_event(value: bool) -> InputEvent {
        InputEvent::new(name("wall_switch"), TypedValue::bool(value))
    }

    fn percent_event(value: u8) -> InputEvent {
        InputEvent::new(name("dimmer_level"), TypedValue::percent(value).unwrap())
    }

    fn timed_effect(value: bool) -> Effect {
        Effect::SetOutput {
            endpoint: name("test_light"),
            value: TypedValue::bool(value),
        }
    }

    fn percent_effect(value: u8) -> Effect {
        Effect::SetOutput {
            endpoint: name("dimmer_output"),
            value: TypedValue::percent(value).unwrap(),
        }
    }

    #[test]
    fn endpoint_names_are_validated() {
        assert!("wall_switch".parse::<EndpointName>().is_ok());
        assert!("a.b-2".parse::<EndpointName>().is_ok());
        assert!("".parse::<EndpointName>().is_err());
        assert!("Wall_switch".parse::<EndpointName>().is_err());
        assert!("1st_input".parse::<EndpointName>().is_err());
        assert!("wall/switch".parse::<EndpointName>().is_err());
    }

    #[test]
    fn dpts_and_values_are_typed() {
        assert_eq!(Dpt::BOOL.to_string(), "1.001");
        assert_eq!(Dpt::PERCENT.to_string(), "5.001");
        assert_eq!(
            TypedValue::new(Dpt::BOOL, Value::Bool(true)).unwrap(),
            TypedValue::bool(true)
        );
        assert_eq!(TypedValue::percent(42).unwrap().value, Value::Percent(42));
        assert!(TypedValue::new(Dpt::BOOL, Value::Percent(42)).is_err());
        assert!(TypedValue::new(Dpt::PERCENT, Value::Bool(true)).is_err());
        assert!(TypedValue::new(Dpt::PERCENT, Value::Percent(101)).is_err());
        assert!(
            TypedValue::new(
                Dpt {
                    major: 9,
                    subtype: 1
                },
                Value::Bool(true)
            )
            .is_err()
        );
    }

    #[test]
    fn configuration_rejects_duplicates_unsupported_dpts_and_bad_rules() {
        let mut invalid = config();
        invalid.endpoints.push(endpoint(
            "wall_switch",
            EndpointDirection::Output,
            Dpt::BOOL,
        ));
        assert!(matches!(
            invalid.validate(),
            Err(ConfigError::DuplicateEndpoint(endpoint)) if endpoint == name("wall_switch")
        ));

        let mut invalid = config();
        invalid.endpoints[0].dpt = Dpt {
            major: 9,
            subtype: 1,
        };
        assert!(matches!(
            invalid.validate(),
            Err(ConfigError::UnsupportedEndpointDpt { endpoint, .. }) if endpoint == name("wall_switch")
        ));

        let mut invalid = config();
        invalid.timed_bool.input = name("missing");
        assert!(matches!(
            invalid.validate(),
            Err(ConfigError::UnknownRuleEndpoint {
                rule: "timed_bool",
                role: "input",
                ..
            })
        ));

        let mut invalid = config();
        invalid.timed_bool.input = name("test_light");
        assert!(matches!(
            invalid.validate(),
            Err(ConfigError::WrongRuleDirection {
                rule: "timed_bool",
                role: "input",
                ..
            })
        ));

        let mut invalid = config();
        invalid.timed_bool.input = name("dimmer_level");
        assert!(matches!(
            invalid.validate(),
            Err(ConfigError::WrongRuleDpt {
                rule: "timed_bool",
                role: "input",
                ..
            })
        ));

        let mut invalid = config();
        invalid.timed_bool.off_delay_ms = 0;
        assert_eq!(invalid.validate(), Err(ConfigError::ZeroOffDelay));
        invalid.timed_bool.off_delay_ms = MAX_OFF_DELAY_MS + 1;
        assert!(matches!(
            invalid.validate(),
            Err(ConfigError::OffDelayTooLarge { .. })
        ));
    }

    #[test]
    fn timed_boolean_triggers_retriggers_and_expires() {
        let mut engine = Engine::new(config());
        assert_eq!(engine.snapshot(), EngineSnapshot { off_deadline: None });

        assert_eq!(
            engine.handle_event(timed_event(true), MonotonicMs(1_000)),
            Ok(vec![timed_effect(true)])
        );
        assert_eq!(
            engine.snapshot(),
            EngineSnapshot {
                off_deadline: Some(MonotonicMs(6_000))
            }
        );
        assert!(engine.poll(MonotonicMs(5_999)).is_empty());

        assert_eq!(
            engine.handle_event(timed_event(true), MonotonicMs(4_000)),
            Ok(vec![timed_effect(true)])
        );
        assert_eq!(
            engine.snapshot(),
            EngineSnapshot {
                off_deadline: Some(MonotonicMs(9_000))
            }
        );
        assert!(engine.poll(MonotonicMs(6_000)).is_empty());
        assert_eq!(engine.poll(MonotonicMs(9_000)), vec![timed_effect(false)]);
        assert_eq!(engine.snapshot(), EngineSnapshot { off_deadline: None });
        assert!(engine.poll(MonotonicMs(9_001)).is_empty());
    }

    #[test]
    fn timed_false_does_not_trigger_or_cancel_timer() {
        let mut engine = Engine::new(config());
        engine
            .handle_event(timed_event(true), MonotonicMs(1_000))
            .unwrap();
        assert!(
            engine
                .handle_event(timed_event(false), MonotonicMs(2_000))
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            engine.snapshot(),
            EngineSnapshot {
                off_deadline: Some(MonotonicMs(6_000))
            }
        );
    }

    #[test]
    fn percentage_forwards_zero_intermediate_and_maximum() {
        let mut engine = Engine::new(config());
        for value in [0, 42, 100] {
            assert_eq!(
                engine.handle_event(percent_event(value), MonotonicMs(1_000)),
                Ok(vec![percent_effect(value)])
            );
            assert_eq!(engine.snapshot(), EngineSnapshot { off_deadline: None });
        }
    }

    #[test]
    fn unused_inputs_produce_no_effects() {
        let mut engine = Engine::new(config());
        assert_eq!(
            engine.handle_event(
                InputEvent::new(name("unused_input"), TypedValue::bool(true)),
                MonotonicMs(1_000)
            ),
            Ok(Vec::new())
        );
        assert_eq!(engine.snapshot(), EngineSnapshot { off_deadline: None });
    }

    #[test]
    fn events_reject_unknown_output_wrong_dpt_and_invalid_value() {
        let mut engine = Engine::new(config());
        assert!(matches!(
            engine.handle_event(
                InputEvent::new(name("test_light"), TypedValue::bool(true)),
                MonotonicMs(0)
            ),
            Err(EventError::EndpointNotInput { .. })
        ));
        assert!(matches!(
            engine.handle_event(
                InputEvent::new(name("wall_switch"), TypedValue::percent(42).unwrap()),
                MonotonicMs(0)
            ),
            Err(EventError::DptMismatch { .. })
        ));
        let invalid_value = TypedValue {
            dpt: Dpt::PERCENT,
            value: Value::Percent(101),
        };
        assert!(matches!(
            engine.handle_event(
                InputEvent::new(name("dimmer_level"), invalid_value),
                MonotonicMs(0)
            ),
            Err(EventError::InvalidValue(ValueError::PercentOutOfRange(101)))
        ));
        assert!(matches!(
            engine.handle_event(
                InputEvent::new(name("missing"), TypedValue::bool(true)),
                MonotonicMs(0)
            ),
            Err(EventError::UnknownEndpoint(endpoint)) if endpoint == name("missing")
        ));
    }
}
