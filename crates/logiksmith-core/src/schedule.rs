//! Milestone 9: schedule engine and captured wall-clock time context.
//!
//! This module is platform-independent (no OS clock, no Tokio, no filesystem)
//! and deterministic: every occurrence is a pure function of the configured
//! rule, the site, and a UTC-unix-millisecond instant. Hosts drive the engine
//! with [`ClockSample`] values built from their own clock sources.
//!
//! Semantics:
//! - Fixed rules fire at a local wall-clock time on matching weekdays.
//! - Interval rules use an absolute UTC phase.
//! - Astronomical rules resolve solar events in the configured site timezone;
//!   weekday filtering is the only schedule-side calendar condition.

include!("schedule_rules.rs");
include!("schedule_astronomy.rs");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        BlockActivation, BlockConfig, BlockConfigError, Dpt, Endpoint, Engine, EngineConfig,
        InputEvent, Runtime, RuntimeActivation, RuntimeConfig, StateValue, Trigger, TypedValue,
    };
    include!("schedule_rule_tests.rs");
    include!("schedule_runtime_tests.rs");
}
