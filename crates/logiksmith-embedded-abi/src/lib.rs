//! Small, versioned C ABI for hosting the portable LogikSmith runtime.
//!
//! The ABI deliberately stops at logical endpoint effects. An OpenKNX host
//! owns group-address bindings and translates each effect to a KNX telegram;
//! no KNX or OpenKNX type is used here. Strings crossing the boundary are
//! pointer/length byte slices and outputs use caller-owned fixed records.

use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    slice, str,
};

use logiksmith_core::{
    BlockConfig, BlockId, Dpt, Endpoint, EndpointDirection, EndpointName, InputEvent,
    InputSnapshot, MonotonicMs, Runtime, RuntimeActivation, RuntimeConfig, RuntimeEventError,
    SimulationInput, SimulationScenario, SimulationTrigger, TimerSimulationScenario, TypedValue,
    Value,
};
use logiksmith_runtime_api::{RuntimeApi, parse_automation_toml};

pub const ABI_VERSION: u32 = 2;
pub const MAX_IDENTIFIER_BYTES: usize = 64;
pub const MAX_SOURCE_BYTES: usize = logiksmith_core::MAX_LOGIC_SOURCE_BYTES;
pub const MAX_BLOCKS: usize = 64;
pub const MAX_ENDPOINTS_PER_BLOCK: usize = 64;
pub const MAX_EFFECTS: usize = 256;
pub const MAX_INPUTS_PER_BLOCK: usize = MAX_ENDPOINTS_PER_BLOCK;
pub const MAX_TIMERS_PER_BLOCK: usize = logiksmith_core::MAX_PENDING_TIMERS;
pub const MAX_BLOCK_PROJECTION_BYTES: usize = 16 * 1024;
pub const MAX_DOCUMENT_BYTES: usize = 16 * 1024;

// The ESP32 Arduino linker does not pull Rust's panic runtime as a separate
// crate when this library is embedded in a C++ image. The abort profile still
// leaves a personality reference in Rust `.eh_frame` records, so provide the
// required no-op symbol from the same archive. No unwinding is enabled for the
// embedded build.
#[cfg(target_arch = "xtensa")]
#[unsafe(no_mangle)]
pub extern "C" fn rust_eh_personality() {}

const INPUT: u8 = 0;
const OUTPUT: u8 = 1;
const VALUE_BOOL: u8 = 1;
const VALUE_PERCENT: u8 = 2;
const VALUE_TEMPERATURE_CENTI_DEGREES: u8 = 3;
const MAX_KNX_GROUP_ADDRESS: u16 = 0x7fff;

include!("abi_records.rs");
include!("abi_exports.rs");
include!("abi_helpers.rs");
include!("abi_tests.rs");
