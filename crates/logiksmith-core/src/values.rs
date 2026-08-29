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
