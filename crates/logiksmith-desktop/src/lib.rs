//! Tokio desktop host for the platform-independent LogikSmith engine.

mod automation;
mod configuration;
pub mod diagnostics;
mod host;
mod protocol;
mod simulation;
pub mod web;

pub use automation::*;
pub use configuration::*;
pub use host::*;
pub use protocol::*;
pub use simulation::*;

pub(crate) mod wire_revision {
    use serde::{Deserialize, Deserializer, Serializer, de::Error};

    pub(crate) struct Value(pub u64);

    impl serde::Serialize for Value {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            serialize(&self.0, serializer)
        }
    }

    pub fn serialize<S>(value: &u64, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&value.to_string())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<u64, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value
            .parse()
            .map_err(|_| D::Error::custom("must be a decimal revision string"))
    }

    pub fn deserialize_option<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Option::<String>::deserialize(deserializer)?;
        value
            .map(|value| {
                value
                    .parse()
                    .map_err(|_| D::Error::custom("must be a decimal revision string"))
            })
            .transpose()
    }
}

pub const MAX_SCHEDULES_PER_BLOCK: usize = 32;
pub const PROTOCOL_VERSION: u64 = 1;
pub const MAX_BLOCKS: usize = 64;

#[cfg(test)]
mod milestone7_simulation_tests {
    use super::*;
    include!("simulation_tests.rs");
}

#[cfg(test)]
mod milestone8_config_tests {
    use super::*;
    use crate::configuration::{RawConfig, parse_duration_seconds, schedule_rule};
    use std::{fs, path::Path};
    include!("configuration_tests.rs");
}
