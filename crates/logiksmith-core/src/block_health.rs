/// A public view of one block's current semantic state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlockSnapshot {
    pub id: BlockId,
    pub enabled: bool,
    pub health: BlockHealth,
    pub consecutive_failures: u32,
    pub live_executions_in_window: usize,
    pub last_suspension: Option<BlockSuspension>,
    pub logic_revision: LogicRevision,
    pub inputs: Vec<InputSnapshot>,
    pub known_inputs: Vec<(EndpointName, TypedValue)>,
    pub state: TransientState,
    pub pending_timers: Vec<PendingTimer>,
}

/// Operational state of a block in the running runtime. This is deliberately
/// separate from [`BlockConfig::enabled`], which is persisted configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlockHealth {
    Disabled,
    Active,
    SuspendedScriptFailures,
    SuspendedEventRate,
}

impl BlockHealth {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Active => "active",
            Self::SuspendedScriptFailures => "suspended_script_failures",
            Self::SuspendedEventRate => "suspended_event_rate",
        }
    }
}

impl fmt::Display for BlockHealth {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Why a block was automatically suspended. The block health enum is the
/// current state; this value is retained for an operations view.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlockSuspension {
    ScriptFailures,
    EventRate,
}
