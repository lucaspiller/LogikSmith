/// The isolated block-level engine owned by [`Runtime`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LogicBlock {
    config: BlockConfig,
    engine: Engine,
}
impl LogicBlock {
    fn try_new(config: BlockConfig) -> Result<Self, RuntimeConfigError> {
        let id = config.id.clone();
        config
            .validate()
            .map_err(|error| RuntimeConfigError::InvalidBlock {
                block_id: id,
                error,
            })?;
        let engine_config =
            EngineConfig::with_program(config.endpoints.clone(), config.logic.clone());
        let engine =
            Engine::try_new(engine_config).map_err(|error| RuntimeConfigError::InvalidBlock {
                block_id: config.id.clone(),
                error: BlockConfigError::Engine(error),
            })?;
        Ok(Self { config, engine })
    }

    pub fn id(&self) -> &BlockId {
        &self.config.id
    }

    pub fn enabled(&self) -> bool {
        self.config.enabled
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
        BlockSnapshot {
            id: self.config.id.clone(),
            enabled: self.config.enabled,
            logic_revision: snapshot.logic_revision,
            inputs: self.engine.input_snapshots(now),
            known_inputs: snapshot.known_inputs,
            state: snapshot.state,
            pending_timers: snapshot.pending_timers,
        }
    }
}

/// Portable serial runtime for up to 64 isolated logic blocks.
#[derive(Clone, Debug, PartialEq)]
pub struct Runtime {
    blocks: Vec<LogicBlock>,
    last_accepted_at: Option<MonotonicMs>,
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
