//! Platform-independent event processing for LogikSmith.
//!
//! The core deals in named, typed endpoints. Hosts provide observations and
//! triggering input events, then execute the logical effects returned by the
//! active Lua program. Transport details such as KNX group addresses stay
//! outside this crate.

mod blocks;
mod engine;
mod engine_config;
mod identifiers;
mod lua;
mod noaa;
mod program;
mod runtime;
pub mod schedule;
mod state;
mod support;
mod values;

pub use blocks::*;
pub use engine::*;
pub use engine_config::*;
pub(crate) use identifiers::validate_endpoint_name;
pub use identifiers::*;
pub use program::*;
pub use runtime::*;
pub use schedule::*;
pub use state::*;
pub use values::*;

pub const MAX_LOGIC_SOURCE_BYTES: usize = 64 * 1024;
pub const MAX_LOGIC_INSTRUCTIONS: u32 = 100_000;
pub const MAX_LOGIC_MEMORY_BYTES: usize = 1024 * 1024;

const INSTRUCTION_LIMIT_MARKER: &str = "logiksmith instruction limit exceeded";

#[cfg(test)]
mod milestone7_tests {
    use super::*;
    include!("legacy_tests.rs");
}

#[cfg(test)]
mod tests {
    use super::*;
    include!("engine_tests.rs");
}

#[cfg(test)]
mod runtime_tests {
    use super::*;
    include!("runtime_tests.rs");
}

#[cfg(test)]
mod signal_tests {
    use super::*;
    include!("signal_tests.rs");
}

#[cfg(test)]
mod milestone11_tests {
    include!("milestone11_tests.rs");
}
