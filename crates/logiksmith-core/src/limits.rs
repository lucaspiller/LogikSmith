//! Runtime capacity and timing contracts shared by every host.
//!
//! The desktop host normally uses [`RuntimeProfile::Desktop`].  The
//! embedded-baseline profile is deliberately conservative and is exercised
//! on desktop before it is used by an OpenKNX host.

/// The immutable set of runtime budgets selected by a host.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuntimeLimits {
    pub max_logic_blocks: usize,
    pub max_signals: usize,
    pub max_signal_bindings: usize,
    pub max_endpoints_per_block: usize,
    pub max_schedules_per_block: usize,
    pub max_logic_source_bytes_per_block: usize,
    pub max_logic_source_bytes_total: usize,
    pub max_logic_instructions: u32,
    pub max_logic_memory_bytes: usize,
    pub logic_handler_time_budget_ms: Option<u64>,
    pub signal_cascade_time_budget_ms: Option<u64>,
    pub openknx_loop_warning_threshold_ms: Option<u64>,
    pub max_state_entries_per_block: usize,
    pub max_state_bytes_per_block: usize,
    pub max_state_bytes_total: usize,
    pub max_pending_timers_per_block: usize,
    pub max_pending_timers_total: usize,
    pub max_output_effects_per_execution: usize,
    pub max_timer_effects_per_execution: usize,
    pub max_state_patch_entries_per_execution: usize,
    pub max_combined_effects_per_execution: usize,
    pub max_cascade_executions: usize,
    pub max_live_executions_per_block_per_second: usize,
}

impl RuntimeLimits {
    pub const fn desktop() -> Self {
        RuntimeProfile::Desktop.limits()
    }

    pub const fn embedded_baseline() -> Self {
        RuntimeProfile::EmbeddedBaseline.limits()
    }

    pub const fn for_profile(profile: RuntimeProfile) -> Self {
        profile.limits()
    }
}

/// Built-in runtime budget sets. Hosts may copy one and adjust it for tests,
/// but production selection is expected to use one of these named profiles.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RuntimeProfile {
    #[default]
    Desktop,
    EmbeddedBaseline,
}

impl RuntimeProfile {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Desktop => "desktop",
            Self::EmbeddedBaseline => "embedded-baseline",
        }
    }

    pub const fn limits(self) -> RuntimeLimits {
        match self {
            Self::Desktop => RuntimeLimits {
                max_logic_blocks: 64,
                max_signals: 256,
                max_signal_bindings: 256,
                max_endpoints_per_block: 128,
                max_schedules_per_block: 32,
                max_logic_source_bytes_per_block: 64 * 1024,
                max_logic_source_bytes_total: 1024 * 1024,
                max_logic_instructions: 100_000,
                max_logic_memory_bytes: 1024 * 1024,
                logic_handler_time_budget_ms: None,
                signal_cascade_time_budget_ms: None,
                openknx_loop_warning_threshold_ms: None,
                max_state_entries_per_block: 64,
                max_state_bytes_per_block: 16 * 1024,
                max_state_bytes_total: 512 * 1024,
                max_pending_timers_per_block: 32,
                max_pending_timers_total: 512,
                max_output_effects_per_execution: 64,
                max_timer_effects_per_execution: 32,
                max_state_patch_entries_per_execution: 64,
                max_combined_effects_per_execution: 128,
                max_cascade_executions: 256,
                max_live_executions_per_block_per_second: 128,
            },
            Self::EmbeddedBaseline => RuntimeLimits {
                max_logic_blocks: 32,
                max_signals: 64,
                max_signal_bindings: 128,
                max_endpoints_per_block: 32,
                max_schedules_per_block: 8,
                max_logic_source_bytes_per_block: 8 * 1024,
                max_logic_source_bytes_total: 128 * 1024,
                max_logic_instructions: 25_000,
                max_logic_memory_bytes: 128 * 1024,
                logic_handler_time_budget_ms: Some(3),
                signal_cascade_time_budget_ms: Some(4),
                openknx_loop_warning_threshold_ms: Some(7),
                max_state_entries_per_block: 16,
                max_state_bytes_per_block: 2 * 1024,
                max_state_bytes_total: 32 * 1024,
                max_pending_timers_per_block: 8,
                max_pending_timers_total: 64,
                max_output_effects_per_execution: 16,
                max_timer_effects_per_execution: 8,
                max_state_patch_entries_per_execution: 16,
                max_combined_effects_per_execution: 32,
                max_cascade_executions: 64,
                max_live_executions_per_block_per_second: 32,
            },
        }
    }
}

/// Capabilities compiled into a binary. These are immutable and describe the
/// binary, not its mutable automation configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompiledFeatures {
    pub timezones: bool,
    pub astronomy: bool,
    pub http_inputs: bool,
    pub webhook_inputs: bool,
}

pub const fn compiled_features() -> CompiledFeatures {
    CompiledFeatures {
        timezones: cfg!(feature = "timezones"),
        astronomy: cfg!(feature = "astronomy"),
        http_inputs: cfg!(feature = "http-inputs"),
        webhook_inputs: cfg!(feature = "webhook-inputs"),
    }
}

/// A host-owned monotonic elapsed-time source used by embedded timing guards.
pub trait BudgetProbe: Send + Sync + 'static {
    /// Elapsed milliseconds since the enclosing handler or cascade began.
    fn elapsed_ms(&self) -> u64;
}

pub type BudgetProbeHandle = std::sync::Arc<dyn BudgetProbe>;

/// A no-op timing source used by the desktop profile and legacy APIs.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopBudgetProbe;

impl BudgetProbe for NoopBudgetProbe {
    fn elapsed_ms(&self) -> u64 {
        0
    }
}
