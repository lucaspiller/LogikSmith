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

pub(crate) fn validate_endpoint_name(value: &str) -> Result<(), EndpointNameError> {
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

/// A stable identity for one configured logic block.
///
/// Block identifiers deliberately use a smaller grammar than endpoint names:
/// they are lowercase ASCII machine labels and may contain digits and `_`.
/// The byte limit is checked explicitly so the portable core has the same
/// bound on every host.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BlockId(String);

impl BlockId {
    pub fn new(value: impl Into<String>) -> Result<Self, BlockIdError> {
        let value = value.into();
        validate_block_id(&value)?;
        Ok(Self(value))
    }

    pub fn parse(value: &str) -> Result<Self, BlockIdError> {
        value.parse()
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for BlockId {
    type Err = BlockIdError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl fmt::Display for BlockId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BlockIdError {
    Empty,
    TooLong { actual: usize, maximum: usize },
    InvalidStart(char),
    InvalidCharacter(char),
}

impl fmt::Display for BlockIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("block ID must not be empty"),
            Self::TooLong { actual, maximum } => {
                write!(
                    formatter,
                    "block ID is {actual} bytes; maximum is {maximum}"
                )
            }
            Self::InvalidStart(character) => write!(
                formatter,
                "block ID must start with a lowercase ASCII letter, got {character:?}"
            ),
            Self::InvalidCharacter(character) => write!(
                formatter,
                "block ID contains invalid character {character:?}; only lowercase ASCII letters, digits, and '_' are allowed"
            ),
        }
    }
}

impl Error for BlockIdError {}

pub const MAX_BLOCK_ID_BYTES: usize = 64;
pub const MAX_BLOCKS: usize = 64;

fn validate_block_id(value: &str) -> Result<(), BlockIdError> {
    if value.is_empty() {
        return Err(BlockIdError::Empty);
    }
    if value.len() > MAX_BLOCK_ID_BYTES {
        return Err(BlockIdError::TooLong {
            actual: value.len(),
            maximum: MAX_BLOCK_ID_BYTES,
        });
    }
    let mut chars = value.chars();
    let first = chars.next().expect("empty block ID handled above");
    if !first.is_ascii_lowercase() {
        return Err(BlockIdError::InvalidStart(first));
    }
    for character in chars {
        if !(character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_') {
            return Err(BlockIdError::InvalidCharacter(character));
        }
    }
    Ok(())
}
