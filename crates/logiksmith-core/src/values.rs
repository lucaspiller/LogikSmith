use std::{error::Error, fmt, str::FromStr};

use crate::EndpointName;

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
    major: u16,
    subtype: u16,
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

    /// DPT 9.001 temperature in degrees Celsius.
    pub const TEMPERATURE: Self = Self {
        major: 9,
        subtype: 1,
    };

    /// Short alias for the DPT 9.001 temperature type.
    pub const TEMP: Self = Self::TEMPERATURE;

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

    pub const fn major(self) -> u16 {
        self.major
    }

    pub const fn subtype(self) -> u16 {
        self.subtype
    }

    pub const fn is_bool(self) -> bool {
        self.major == Self::BOOL.major && self.subtype == Self::BOOL.subtype
    }

    pub const fn is_percent(self) -> bool {
        self.major == Self::PERCENT.major && self.subtype == Self::PERCENT.subtype
    }

    pub const fn is_temperature(self) -> bool {
        self.major == Self::TEMPERATURE.major && self.subtype == Self::TEMPERATURE.subtype
    }

    pub const fn is_supported(self) -> bool {
        self.is_bool() || self.is_percent() || self.is_temperature()
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
    /// DPT 9.001 signed hundredths of a degree Celsius.
    Temperature(i32),
}

/// A value with its DPT identity attached.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TypedValue {
    dpt: Dpt,
    value: Value,
}

impl TypedValue {
    pub fn new(dpt: Dpt, value: Value) -> Result<Self, ValueError> {
        match (dpt, value) {
            (dpt, Value::Bool(_)) if dpt.is_bool() => Ok(Self { dpt, value }),
            (dpt, Value::Percent(value)) if dpt.is_percent() && value <= 100 => Ok(Self {
                dpt,
                value: Value::Percent(value),
            }),
            (dpt, Value::Percent(value)) if dpt.is_percent() => {
                Err(ValueError::PercentOutOfRange(value))
            }
            (dpt, Value::Temperature(value)) if dpt.is_temperature() => Ok(Self {
                dpt,
                value: Value::Temperature(value),
            }),
            (dpt, _) if !dpt.is_supported() => Err(ValueError::UnsupportedDpt(dpt)),
            (dpt, value) => Err(ValueError::DptValueMismatch { dpt, value }),
        }
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

    /// Creates a DPT 9.001 value from signed hundredths of a degree Celsius.
    ///
    /// Keeping the canonical representation integral makes equality, signal
    /// propagation, and snapshots deterministic across hosts.
    pub fn temperature_centi_degrees(value: i32) -> Result<Self, ValueError> {
        Self::new(Dpt::TEMPERATURE, Value::Temperature(value))
    }

    /// Creates a DPT 9.001 value from degrees Celsius. Values must be finite
    /// and have no more than two decimal places.
    pub fn temperature(value: f64) -> Result<Self, ValueError> {
        if !value.is_finite() {
            return Err(ValueError::TemperatureNotFinite);
        }
        let scaled = value * 100.0;
        let rounded = scaled.round();
        // The tolerance only absorbs ordinary binary floating point noise for
        // decimal values such as 12.34. A third decimal place remains well
        // outside this tolerance and is rejected.
        if (scaled - rounded).abs() > 1e-9 {
            return Err(ValueError::TemperaturePrecision);
        }
        if rounded < i32::MIN as f64 || rounded > i32::MAX as f64 {
            return Err(ValueError::TemperatureOutOfRange);
        }
        Self::temperature_centi_degrees(rounded as i32)
    }

    /// Explicit spelling for callers converting a host's Celsius scalar.
    pub fn temperature_celsius(value: f64) -> Result<Self, ValueError> {
        Self::temperature(value)
    }

    /// Returns the canonical signed hundredths-of-a-degree representation.
    pub const fn temperature_centi(self) -> Option<i32> {
        match self.value {
            Value::Temperature(value) if self.dpt.is_temperature() => Some(value),
            _ => None,
        }
    }

    /// Returns a DPT 9.001 value in degrees Celsius.
    pub fn temperature_celsius_value(self) -> Option<f64> {
        self.temperature_centi()
            .map(|value| f64::from(value) / 100.0)
    }

    pub const fn dpt(self) -> Dpt {
        self.dpt
    }

    pub const fn value(self) -> Value {
        self.value
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValueError {
    UnsupportedDpt(Dpt),
    DptValueMismatch { dpt: Dpt, value: Value },
    PercentOutOfRange(u8),
    TemperatureNotFinite,
    TemperaturePrecision,
    TemperatureOutOfRange,
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
            Self::TemperatureNotFinite => formatter.write_str("temperature must be finite"),
            Self::TemperaturePrecision => {
                formatter.write_str("temperature must have at most two decimal places")
            }
            Self::TemperatureOutOfRange => {
                formatter.write_str("temperature is outside the signed centi-degree range")
            }
        }
    }
}

impl Error for ValueError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dpt_components_are_read_only_value_object_data() {
        let dpt = Dpt::new(9, 4).unwrap();
        assert_eq!(dpt.major(), 9);
        assert_eq!(dpt.subtype(), 4);
        assert_eq!(dpt.to_string(), "9.004");
    }

    #[test]
    fn typed_value_constructor_preserves_dpt_pairing_invariant() {
        assert!(TypedValue::new(Dpt::BOOL, Value::Percent(42)).is_err());
        assert!(TypedValue::new(Dpt::PERCENT, Value::Percent(101)).is_err());

        let value = TypedValue::bool(true);
        assert_eq!(value.dpt(), Dpt::BOOL);
        assert_eq!(value.value(), Value::Bool(true));
    }
}

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

/// A transport-neutral input update supplied by a host.
///
/// `Observe` refreshes a known value without evaluating Lua. `Trigger` stores
/// the value and evaluates the block. `Invalidate` clears the known value
/// without evaluating Lua, leaving the input unknown until a later update.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputUpdate {
    Observe(TypedValue),
    Trigger(TypedValue),
    Invalidate,
}

impl InputObservation {
    pub fn new(endpoint: EndpointName, value: TypedValue) -> Self {
        Self { endpoint, value }
    }
}

/// A logical output effect for the host to execute.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutputEffect {
    pub endpoint: EndpointName,
    pub value: TypedValue,
}

impl OutputEffect {
    pub fn new(endpoint: EndpointName, value: TypedValue) -> Self {
        Self { endpoint, value }
    }
}

/// Compatibility alias for the Milestone 5 output name.
pub type Effect = OutputEffect;

/// A host-provided monotonic timestamp retained as a small transport-neutral
/// value for core-owned timer deadlines and desktop diagnostics.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct MonotonicMs(pub u64);

impl MonotonicMs {
    pub fn checked_add(self, milliseconds: u32) -> Option<Self> {
        self.0.checked_add(u64::from(milliseconds)).map(Self)
    }
}

/// A deterministic revision derived from the exact source bytes.
///
/// This deliberately uses the same compact scalar representation as the
/// desktop's document/content revisions, while keeping revision calculation
/// inside the core's source-program boundary.
pub type LogicRevision = u64;
