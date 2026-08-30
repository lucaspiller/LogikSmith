/// Five consecutive live execution failures suspend one block until an
/// explicit resume or source activation.
pub const BLOCK_FAILURE_SUSPENSION_THRESHOLD: u32 = 5;

/// The isolated block-level engine owned by [`Runtime`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LogicBlock {
    config: BlockConfig,
    engine: Engine,
    health: BlockHealth,
    consecutive_failures: u32,
    live_execution_times: VecDeque<MonotonicMs>,
    last_suspension: Option<BlockSuspension>,
}
impl LogicBlock {
    fn try_new(
        config: BlockConfig,
        limits: &RuntimeLimits,
    ) -> Result<Self, RuntimeConfigError> {
        let id = config.id.clone();
        config
            .validate_with_limits(limits)
            .map_err(|error| RuntimeConfigError::InvalidBlock {
                block_id: id,
                error,
            })?;
        let engine_config =
            EngineConfig::with_program(config.endpoints.clone(), config.logic.clone());
        let engine =
            Engine::try_new_with_limits(engine_config, *limits).map_err(|error| RuntimeConfigError::InvalidBlock {
                block_id: config.id.clone(),
                error: BlockConfigError::Engine(error),
            })?;
        let health = if config.enabled {
            BlockHealth::Active
        } else {
            BlockHealth::Disabled
        };
        Ok(Self {
            config,
            engine,
            health,
            consecutive_failures: 0,
            live_execution_times: VecDeque::new(),
            last_suspension: None,
        })
    }

    pub fn id(&self) -> &BlockId {
        &self.config.id
    }

    pub fn enabled(&self) -> bool {
        self.config.enabled
    }

    pub fn health(&self) -> BlockHealth {
        self.health
    }

    pub fn consecutive_failures(&self) -> u32 {
        self.consecutive_failures
    }

    pub fn live_executions_in_window(&self) -> usize {
        self.live_execution_times.len()
    }

    pub fn last_suspension(&self) -> Option<BlockSuspension> {
        self.last_suspension
    }

    pub fn config(&self) -> &BlockConfig {
        &self.config
    }

    pub fn logic_program(&self) -> &LogicProgram {
        self.engine.logic_program()
    }

    pub fn active_logic_revision(&self) -> LogicRevision {
        self.engine.active_logic_revision()
    }

    pub fn snapshot_at(&self, now: MonotonicMs) -> BlockSnapshot {
        let snapshot = self.engine.snapshot();
        let live_executions_in_window = self
            .live_execution_times
            .iter()
            .filter(|started| now.0.saturating_sub(started.0) < 1_000)
            .count();
        BlockSnapshot {
            id: self.config.id.clone(),
            enabled: self.config.enabled,
            health: self.health,
            consecutive_failures: self.consecutive_failures,
            live_executions_in_window,
            last_suspension: self.last_suspension,
            logic_revision: snapshot.logic_revision,
            inputs: self.engine.input_snapshots(now),
            known_inputs: snapshot.known_inputs,
            state: snapshot.state,
            pending_timers: snapshot.pending_timers,
        }
    }

    fn prune_live_execution_window(&mut self, now: MonotonicMs) {
        while self
            .live_execution_times
            .front()
            .is_some_and(|started| now.0.saturating_sub(started.0) >= 1_000)
        {
            self.live_execution_times.pop_front();
        }
    }

    /// Admits one live handler invocation. A rejected invocation is observed
    /// by the caller and never enters Lua.
    fn admit_live_execution(&mut self, now: MonotonicMs, maximum: usize) -> bool {
        if self.health != BlockHealth::Active {
            return false;
        }
        self.prune_live_execution_window(now);
        if self.live_execution_times.len() >= maximum {
            self.health = BlockHealth::SuspendedEventRate;
            self.last_suspension = Some(BlockSuspension::EventRate);
            self.consecutive_failures = 0;
            self.engine.pending_timers.clear();
            return false;
        }
        self.live_execution_times.push_back(now);
        true
    }

    /// Records only live handler outcomes. Simulation never reaches this
    /// method, so it cannot consume rate budget or count towards suspension.
    fn record_live_execution(&mut self, execution: &Execution) {
        let failed = execution.outcome.as_ref().err().is_some_and(|error| {
            matches!(
                error.kind(),
                LogicErrorKind::Runtime
                    | LogicErrorKind::InstructionLimit
                    | LogicErrorKind::MemoryLimit
                    | LogicErrorKind::HandlerTimeLimit
                    | LogicErrorKind::InvalidResult
            )
        });
        if failed {
            self.consecutive_failures = self.consecutive_failures.saturating_add(1);
            if self.consecutive_failures >= BLOCK_FAILURE_SUSPENSION_THRESHOLD {
                self.health = BlockHealth::SuspendedScriptFailures;
                self.last_suspension = Some(BlockSuspension::ScriptFailures);
                self.live_execution_times.clear();
                self.engine.pending_timers.clear();
            }
        } else if execution.outcome.is_ok() {
            self.consecutive_failures = 0;
        }
    }

    fn reset_health(&mut self) {
        self.health = if self.config.enabled {
            BlockHealth::Active
        } else {
            BlockHealth::Disabled
        };
        self.consecutive_failures = 0;
        self.live_execution_times.clear();
        self.last_suspension = None;
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.config.enabled = enabled;
        if !enabled {
            self.engine.pending_timers.clear();
        }
        self.reset_health();
    }

    fn resume_health(&mut self) {
        self.reset_health();
    }
}

/// Portable serial runtime for up to 64 isolated logic blocks.
#[derive(Clone, Debug, PartialEq)]
pub struct Runtime {
    pub(crate) limits: RuntimeLimits,
    blocks: Vec<LogicBlock>,
    signals: Vec<SignalState>,
    signal_indexes: BTreeMap<SignalName, usize>,
    last_accepted_at: Option<MonotonicMs>,
    next_execution_id: ExecutionId,
    /// The site wall-clock context is captured from for every execution.
    site: SiteTimeConfig,
    /// Per-block, per-schedule engine state, keyed deterministically by
    /// (block id, schedule name).
    schedule_cursors: BTreeMap<(BlockId, ScheduleName), schedule::ScheduleCursor>,
    /// The structural revision associated with the current schedule set.
    /// This is retained even while the host clock is unavailable so a later
    /// valid sample can establish a baseline without a catch-up burst.
    schedule_structural_revision: Option<u64>,
    /// Last valid wall-clock sample used by the schedule poller. `None`
    /// means the next valid sample must establish a future-only baseline.
    last_schedule_wall_clock_utc_ms: Option<i64>,
}

/// Current bounded live-data usage, useful for host diagnostics and tests.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RuntimeUsage {
    pub logic_blocks: usize,
    pub signals: usize,
    pub signal_bindings: usize,
    pub logic_source_bytes: usize,
    pub state_entries: usize,
    pub state_bytes: usize,
    pub pending_timers: usize,
}

#[derive(Clone, Debug, PartialEq)]
struct SignalState {
    config: SignalConfig,
    value: Option<TypedValue>,
    observed_at: Option<MonotonicMs>,
    changed_at: Option<MonotonicMs>,
    producer: Option<SignalEndpointId>,
    producing_execution: Option<ExecutionId>,
    consumers: Vec<SignalEndpointId>,
}
