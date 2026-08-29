impl Runtime {
    pub fn new(config: RuntimeConfig) -> Self {
        Self::try_new(config).expect("invalid LogikSmith core runtime configuration")
    }

    pub fn try_new(config: RuntimeConfig) -> Result<Self, RuntimeConfigError> {
        config.validate()?;
        let site = config.site.clone();
        let blocks = config
            .blocks
            .into_iter()
            .map(LogicBlock::try_new)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            blocks,
            last_accepted_at: None,
            site,
            schedule_cursors: BTreeMap::new(),
            schedule_structural_revision: None,
            last_schedule_wall_clock_utc_ms: None,
        })
    }

    pub fn block(&self, id: &BlockId) -> Option<&LogicBlock> {
        self.blocks.iter().find(|block| block.id() == id)
    }

    pub fn blocks(&self) -> &[LogicBlock] {
        &self.blocks
    }

    pub fn config(&self) -> RuntimeConfig {
        RuntimeConfig::with_site(
            self.blocks
                .iter()
                .map(|block| block.config.clone())
                .collect(),
            self.site.clone(),
        )
    }

    pub fn block_ids(&self) -> Vec<BlockId> {
        self.blocks.iter().map(|block| block.id().clone()).collect()
    }

    pub fn last_accepted_at(&self) -> Option<MonotonicMs> {
        self.last_accepted_at
    }

    pub fn snapshot(&self) -> RuntimeSnapshot {
        self.snapshot_at(self.last_accepted_at.unwrap_or_default())
    }

    pub fn snapshot_at(&self, now: MonotonicMs) -> RuntimeSnapshot {
        RuntimeSnapshot {
            blocks: self
                .blocks
                .iter()
                .map(|block| block.snapshot_at(now))
                .collect(),
            last_accepted_at: self.last_accepted_at,
        }
    }

    /// Records an input without invoking Lua. This is intended for passive
    /// observations, including observations received while a block is
    /// disabled.
    pub fn observe_input(
        &mut self,
        block_id: &BlockId,
        observation: InputObservation,
        now: MonotonicMs,
    ) -> Result<(), RuntimeEventError> {
        let index = self.block_index(block_id)?;
        self.blocks[index]
            .engine
            .validate_input(&observation.endpoint, observation.value)
            .map_err(|error| RuntimeEventError::Block {
                block_id: block_id.clone(),
                error,
            })?;
        self.ensure_time(Some(block_id), now)?;
        self.blocks[index]
            .engine
            .observe_input(observation, now)
            .map_err(|error| RuntimeEventError::Block {
                block_id: block_id.clone(),
                error,
            })?;
        self.last_accepted_at = Some(now);
        Ok(())
    }

    /// Delivers one triggering input to one block. A disabled block accepts
    /// the observation but returns no semantic execution.
    pub fn process_input(
        &mut self,
        block_id: &BlockId,
        event: InputEvent,
        now: MonotonicMs,
    ) -> Result<Option<BlockExecution>, RuntimeEventError> {
        let index = self.block_index(block_id)?;
        self.blocks[index]
            .engine
            .validate_input(&event.endpoint, event.value)
            .map_err(|error| RuntimeEventError::Block {
                block_id: block_id.clone(),
                error,
            })?;
        self.ensure_time(Some(block_id), now)?;
        if !self.blocks[index].config.enabled {
            self.blocks[index]
                .engine
                .observe_input(InputObservation::new(event.endpoint, event.value), now)
                .map_err(|error| RuntimeEventError::Block {
                    block_id: block_id.clone(),
                    error,
                })?;
            self.last_accepted_at = Some(now);
            return Ok(None);
        }
        let execution = self.blocks[index]
            .engine
            .process_input(event, now)
            .map_err(|error| RuntimeEventError::Block {
                block_id: block_id.clone(),
                error,
            })?;
        self.last_accepted_at = Some(now);
        Ok(Some(BlockExecution {
            block_id: block_id.clone(),
            execution,
        }))
    }

    /// ClockSample variant of [`Self::process_input`]: captures the frozen
    /// time context from the runtime site and the sample's wall-clock instant.
    pub fn process_input_sampled(
        &mut self,
        block_id: &BlockId,
        event: InputEvent,
        sample: ClockSample,
    ) -> Result<Option<BlockExecution>, RuntimeEventError> {
        let index = self.block_index(block_id)?;
        self.blocks[index]
            .engine
            .validate_input(&event.endpoint, event.value)
            .map_err(|error| RuntimeEventError::Block {
                block_id: block_id.clone(),
                error,
            })?;
        self.ensure_time(Some(block_id), sample.monotonic_ms)?;
        if !self.blocks[index].config.enabled {
            self.blocks[index]
                .engine
                .observe_input(
                    InputObservation::new(event.endpoint, event.value),
                    sample.monotonic_ms,
                )
                .map_err(|error| RuntimeEventError::Block {
                    block_id: block_id.clone(),
                    error,
                })?;
            self.last_accepted_at = Some(sample.monotonic_ms);
            return Ok(None);
        }
        let execution = self.blocks[index]
            .engine
            .process_input_sampled(event, sample, &self.site)
            .map_err(|error| RuntimeEventError::Block {
                block_id: block_id.clone(),
                error,
            })?;
        self.last_accepted_at = Some(sample.monotonic_ms);
        Ok(Some(BlockExecution {
            block_id: block_id.clone(),
            execution,
        }))
    }

    pub fn next_timer_deadline(&self) -> Option<MonotonicMs> {
        self.blocks
            .iter()
            .filter(|block| block.enabled())
            .flat_map(|block| block.engine.pending_timers())
            .map(|timer| timer.due_at)
            .min()
    }

    /// Consumes and evaluates at most one due timer using global deterministic
    /// ordering `(deadline, block ID, timer name)`.
    pub fn process_next_due_timer(
        &mut self,
        now: MonotonicMs,
    ) -> Result<Option<BlockExecution>, RuntimeEventError> {
        self.ensure_time(None, now)?;
        let selected = self
            .blocks
            .iter()
            .enumerate()
            .filter(|(_, block)| block.enabled())
            .flat_map(|(index, block)| {
                block
                    .engine
                    .pending_timers()
                    .into_iter()
                    .filter(move |timer| timer.due_at <= now)
                    .map(move |timer| (index, timer))
            })
            .min_by(|(left_index, left), (right_index, right)| {
                left.due_at
                    .cmp(&right.due_at)
                    .then_with(|| {
                        self.blocks[*left_index]
                            .id()
                            .cmp(self.blocks[*right_index].id())
                    })
                    .then_with(|| left.name.cmp(&right.name))
            });
        let Some((index, _timer)) = selected else {
            self.last_accepted_at = Some(now);
            return Ok(None);
        };
        let block_id = self.blocks[index].id().clone();
        let execution = self.blocks[index]
            .engine
            .process_next_due_timer(now)
            .map_err(|error| RuntimeEventError::Block {
                block_id: block_id.clone(),
                error,
            })?;
        self.last_accepted_at = Some(now);
        Ok(execution.map(|execution| BlockExecution {
            block_id,
            execution,
        }))
    }

    pub fn process_due_timer(
        &mut self,
        now: MonotonicMs,
    ) -> Result<Option<BlockExecution>, RuntimeEventError> {
        self.process_next_due_timer(now)
    }

    /// ClockSample variant of [`Self::process_next_due_timer`]: captures the
    /// frozen time context from the runtime site and the sample's wall-clock
    /// instant.
    pub fn process_next_due_timer_sampled(
        &mut self,
        sample: ClockSample,
    ) -> Result<Option<BlockExecution>, RuntimeEventError> {
        self.ensure_time(None, sample.monotonic_ms)?;
        let selected = self
            .blocks
            .iter()
            .enumerate()
            .filter(|(_, block)| block.enabled())
            .flat_map(|(index, block)| {
                block
                    .engine
                    .pending_timers()
                    .into_iter()
                    .filter(move |timer| timer.due_at <= sample.monotonic_ms)
                    .map(move |timer| (index, timer))
            })
            .min_by(|(left_index, left), (right_index, right)| {
                left.due_at
                    .cmp(&right.due_at)
                    .then_with(|| {
                        self.blocks[*left_index]
                            .id()
                            .cmp(self.blocks[*right_index].id())
                    })
                    .then_with(|| left.name.cmp(&right.name))
            });
        let Some((index, _timer)) = selected else {
            self.last_accepted_at = Some(sample.monotonic_ms);
            return Ok(None);
        };
        let block_id = self.blocks[index].id().clone();
        let execution = self.blocks[index]
            .engine
            .process_next_due_timer_sampled(sample, &self.site)
            .map_err(|error| RuntimeEventError::Block {
                block_id: block_id.clone(),
                error,
            })?;
        self.last_accepted_at = Some(sample.monotonic_ms);
        Ok(execution.map(|execution| BlockExecution {
            block_id,
            execution,
        }))
    }

    /// Recomputes the schedule cursors for every configured schedule: the
    /// baseline is `sample` and every next occurrence is strictly after it.
    /// Hosts call this at startup and after any structural schedule change
    /// (including re-enabling a block) so schedules never fire retroactively.
    pub fn initialise_schedules(
        &mut self,
        sample: ClockSample,
        structural_revision: u64,
    ) -> Result<(), TimeError> {
        self.ensure_schedule_time(sample.monotonic_ms)?;
        self.last_accepted_at = Some(sample.monotonic_ms);
        self.schedule_structural_revision = Some(structural_revision);
        self.schedule_cursors.clear();
        let Some(utc) = sample.utc_unix_ms else {
            self.last_schedule_wall_clock_utc_ms = None;
            for block in &self.blocks {
                for block_schedule in &block.config.schedules {
                    self.schedule_cursors.insert(
                        (block.id().clone(), block_schedule.name.clone()),
                        schedule::ScheduleCursor {
                            last_delivered_utc_ms: None,
                            next_occurrence_utc_ms: None,
                            structural_revision,
                            needs_rebaseline: true,
                        },
                    );
                }
            }
            return Err(TimeError::ClockUnavailable);
        };
        self.last_schedule_wall_clock_utc_ms = Some(utc);
        for block in &self.blocks {
            for block_schedule in &block.config.schedules {
                let next = schedule::next_occurrence_after(&block_schedule.rule, &self.site, utc);
                self.schedule_cursors.insert(
                    (block.id().clone(), block_schedule.name.clone()),
                    schedule::ScheduleCursor {
                        last_delivered_utc_ms: None,
                        next_occurrence_utc_ms: next,
                        structural_revision,
                        needs_rebaseline: false,
                    },
                );
            }
        }
        Ok(())
    }

    /// The earliest next occurrence across every enabled block's enabled
    /// schedule, in UTC milliseconds.
    pub fn next_schedule_deadline(&self) -> Option<UtcUnixMs> {
        let mut earliest: Option<i64> = None;
        for block in &self.blocks {
            if !block.enabled() {
                continue;
            }
            for block_schedule in &block.config.schedules {
                if !block_schedule.enabled {
                    continue;
                }
                let key = (block.id().clone(), block_schedule.name.clone());
                if let Some(cursor) = self.schedule_cursors.get(&key)
                    && let Some(next) = cursor.next_occurrence_utc_ms
                {
                    earliest = Some(match earliest {
                        Some(current) => current.min(next),
                        None => next,
                    });
                }
            }
        }
        earliest.map(UtcUnixMs)
    }

    /// Delivers the due occurrences of every enabled schedule (latest-only
    /// coalescing per schedule, `coalesced_count` = passed - 1), recomputes
    /// the cursors strictly after the sample, and returns triggers ordered by
    /// `(scheduled_for_utc_ms, block id, schedule name)`. An invalid wall
    /// clock (UTC `None`) pauses every schedule and yields no triggers.
    pub fn poll_schedules(
        &mut self,
        sample: ClockSample,
    ) -> Result<Vec<ScheduleTrigger>, TimeError> {
        self.ensure_schedule_time(sample.monotonic_ms)?;
        self.last_accepted_at = Some(sample.monotonic_ms);
        let Some(now_utc) = sample.utc_unix_ms else {
            self.last_schedule_wall_clock_utc_ms = None;
            for cursor in self.schedule_cursors.values_mut() {
                cursor.next_occurrence_utc_ms = None;
                cursor.needs_rebaseline = true;
            }
            return Ok(Vec::new());
        };
        let wall_clock_went_backwards = self
            .last_schedule_wall_clock_utc_ms
            .is_some_and(|previous| now_utc < previous);
        let needs_initial_baseline = self.last_schedule_wall_clock_utc_ms.is_none();
        if wall_clock_went_backwards || needs_initial_baseline {
            self.recompute_schedule_cursors(now_utc);
        }
        let mut triggers = Vec::new();
        for index in 0..self.blocks.len() {
            let block = &self.blocks[index];
            if !block.enabled() {
                continue;
            }
            for block_schedule in &block.config.schedules {
                if !block_schedule.enabled {
                    continue;
                }
                let key = (block.id().clone(), block_schedule.name.clone());
                let structural_revision = self.schedule_structural_revision.unwrap_or(0);
                let cursor = self.schedule_cursors.entry(key.clone()).or_insert_with(|| {
                    schedule::ScheduleCursor {
                        last_delivered_utc_ms: None,
                        next_occurrence_utc_ms: None,
                        structural_revision,
                        needs_rebaseline: true,
                    }
                });
                if cursor.needs_rebaseline {
                    cursor.next_occurrence_utc_ms = next_occurrence_after_not_before(
                        &block_schedule.rule,
                        &self.site,
                        now_utc,
                        cursor.last_delivered_utc_ms,
                    );
                    cursor.needs_rebaseline = false;
                }
                let Some(next) = cursor.next_occurrence_utc_ms else {
                    continue;
                };
                if next > now_utc {
                    continue;
                }
                // Coalesce: walk every occurrence that passed since the last
                // cursor, delivering only the latest.
                let mut last = next;
                let mut count: u64 = 1;
                loop {
                    match schedule::next_occurrence_after(&block_schedule.rule, &self.site, last) {
                        Some(occurrence) if occurrence <= now_utc => {
                            last = occurrence;
                            count += 1;
                        }
                        _ => break,
                    }
                }
                triggers.push(ScheduleTrigger {
                    block_id: block.id().clone(),
                    name: block_schedule.name.clone(),
                    kind: block_schedule.rule.kind(),
                    scheduled_for_utc_ms: last,
                    detected_at_utc_ms: now_utc,
                    coalesced_count: count - 1,
                    structural_revision: cursor.structural_revision,
                });
                cursor.last_delivered_utc_ms = Some(last);
                cursor.next_occurrence_utc_ms =
                    schedule::next_occurrence_after(&block_schedule.rule, &self.site, now_utc);
            }
        }
        self.last_schedule_wall_clock_utc_ms = Some(now_utc);
        triggers.sort_by(|left, right| {
            left.scheduled_for_utc_ms
                .cmp(&right.scheduled_for_utc_ms)
                .then_with(|| left.block_id.cmp(&right.block_id))
                .then_with(|| left.name.cmp(&right.name))
        });
        Ok(triggers)
    }

    /// Re-establishes future-only schedule cursors for one block using a fresh
    /// paired host sample. Hosts should call this after re-enabling a block so
    /// occurrences that passed while it was disabled are not delivered. A
    /// valid sample updates only the selected block; an invalid sample marks
    /// that block for lazy rebaseline at the next valid poll.
    pub fn rebaseline_block_schedules(
        &mut self,
        block_id: &BlockId,
        sample: ClockSample,
    ) -> Result<(), TimeError> {
        self.ensure_schedule_time(sample.monotonic_ms)?;
        self.last_accepted_at = Some(sample.monotonic_ms);
        if !self.blocks.iter().any(|block| block.id() == block_id) {
            return Err(TimeError::UnknownBlock(block_id.clone()));
        }
        let structural_revision = self.schedule_structural_revision.unwrap_or(0);
        let schedules: Vec<(ScheduleName, ScheduleRule)> = self
            .blocks
            .iter()
            .find(|block| block.id() == block_id)
            .into_iter()
            .flat_map(|block| {
                block.config.schedules.iter().map(|block_schedule| {
                    (block_schedule.name.clone(), block_schedule.rule.clone())
                })
            })
            .collect();
        for (name, rule) in schedules {
            let key = (block_id.clone(), name);
            let cursor =
                self.schedule_cursors
                    .entry(key)
                    .or_insert_with(|| schedule::ScheduleCursor {
                        last_delivered_utc_ms: None,
                        next_occurrence_utc_ms: None,
                        structural_revision,
                        needs_rebaseline: true,
                    });
            cursor.structural_revision = structural_revision;
            cursor.needs_rebaseline = true;
            cursor.next_occurrence_utc_ms = None;
            if let Some(utc) = sample.utc_unix_ms {
                cursor.next_occurrence_utc_ms = next_occurrence_after_not_before(
                    &rule,
                    &self.site,
                    utc,
                    cursor.last_delivered_utc_ms,
                );
                cursor.needs_rebaseline = false;
            }
        }
        if sample.utc_unix_ms.is_none() {
            self.last_schedule_wall_clock_utc_ms = None;
        }
        Ok(())
    }

    /// Stateless preview of the next `count` occurrences of one schedule
    /// strictly after `after_utc_ms`. Never mutates engine state.
    pub fn preview_occurrences(
        &self,
        block_id: &BlockId,
        schedule: &ScheduleName,
        after_utc_ms: i64,
        count: usize,
    ) -> Result<Vec<ScheduleOccurrence>, ScheduleError> {
        let block = self
            .blocks
            .iter()
            .find(|block| block.id() == block_id)
            .ok_or(ScheduleError::UnknownSchedule)?;
        let block_schedule = block
            .config
            .schedules
            .iter()
            .find(|block_schedule| &block_schedule.name == schedule)
            .ok_or(ScheduleError::UnknownSchedule)?;
        let mut occurrences = Vec::new();
        let mut cursor = after_utc_ms;
        while occurrences.len() < count {
            match schedule::next_occurrence_after(&block_schedule.rule, &self.site, cursor) {
                Some(occurrence) => {
                    occurrences.push(ScheduleOccurrence { utc_ms: occurrence });
                    cursor = occurrence;
                }
                None => break,
            }
        }
        Ok(occurrences)
    }

    /// Runs the block's evaluator for one delivered schedule trigger. Returns
    /// `Ok(None)` without executing Lua for an unknown block, an unknown or
    /// disabled schedule, a disabled block, or a stale structural revision.
    pub fn process_schedule(
        &mut self,
        trigger: ScheduleTrigger,
    ) -> Result<Option<BlockExecution>, RuntimeEventError> {
        let now = self.last_accepted_at.unwrap_or_default();
        self.process_schedule_sampled(
            trigger.clone(),
            ClockSample {
                monotonic_ms: now,
                // The compatibility path has no host wall-clock sample. The
                // occurrence instant still gives the execution its logical
                // schedule context, matching the sampled API.
                utc_unix_ms: Some(trigger.scheduled_for_utc_ms),
            },
        )
    }

    /// Runs one schedule trigger using the paired sample taken by the host
    /// for the scheduler poll. The monotonic component is the execution's
    /// current time, so any timers requested by the schedule are relative to
    /// this handling instant rather than the previous input or timer event.
    pub fn process_schedule_sampled(
        &mut self,
        trigger: ScheduleTrigger,
        sample: ClockSample,
    ) -> Result<Option<BlockExecution>, RuntimeEventError> {
        self.ensure_time(None, sample.monotonic_ms)?;
        self.last_accepted_at = Some(sample.monotonic_ms);
        let Some(index) = self
            .blocks
            .iter()
            .position(|block| block.id() == &trigger.block_id)
        else {
            return Ok(None);
        };
        {
            let block = &self.blocks[index];
            if !block.enabled() {
                return Ok(None);
            }
            let Some(block_schedule) = block
                .config
                .schedules
                .iter()
                .find(|block_schedule| block_schedule.name == trigger.name)
            else {
                return Ok(None);
            };
            if !block_schedule.enabled {
                return Ok(None);
            }
            let Some(cursor) = self
                .schedule_cursors
                .get(&(trigger.block_id.clone(), trigger.name.clone()))
            else {
                return Ok(None);
            };
            if cursor.structural_revision != trigger.structural_revision {
                return Ok(None);
            }
        }
        let scheduled_for = trigger.scheduled_for_utc_ms;
        let block_id = self.blocks[index].id().clone();
        let execution = self.blocks[index]
            .engine
            .process_schedule_trigger(
                trigger,
                &self.site,
                Some(scheduled_for),
                sample.monotonic_ms,
            )
            .map_err(|error| RuntimeEventError::Block {
                block_id: block_id.clone(),
                error,
            })?;
        Ok(Some(BlockExecution {
            block_id,
            execution,
        }))
    }

    /// Validates a simulation request (known schedule, genuine occurrence,
    /// current logic and structural revisions) and evaluates it without
    /// mutating the runtime. The time context is frozen at the occurrence.
    pub fn simulate_schedule(
        &self,
        request: ScheduleSimulationRequest,
    ) -> Result<BlockExecution, ScheduleSimulationError> {
        let block = self
            .blocks
            .iter()
            .find(|block| block.id() == &request.block_id)
            .ok_or(ScheduleSimulationError::UnknownSchedule)?;
        let block_schedule = block
            .config
            .schedules
            .iter()
            .find(|block_schedule| block_schedule.name == request.schedule)
            .ok_or(ScheduleSimulationError::UnknownSchedule)?;
        let is_occurrence = schedule::next_occurrence_after(
            &block_schedule.rule,
            &self.site,
            request.occurrence_at_utc_ms.saturating_sub(1),
        ) == Some(request.occurrence_at_utc_ms);
        if !is_occurrence {
            return Err(ScheduleSimulationError::NotOccurrence);
        }
        let Some(cursor) = self
            .schedule_cursors
            .get(&(request.block_id.clone(), request.schedule.clone()))
        else {
            return Err(ScheduleSimulationError::StaleStructuralRevision);
        };
        // A simulation selection must come from the current preview window,
        // not from an arbitrary historical occurrence. The preview API is
        // stateless, so the cursor's next future occurrence is the lower
        // bound that the core can validate without adding a server token.
        let Some(next_previewable) = cursor.next_occurrence_utc_ms else {
            return Err(ScheduleSimulationError::NotOccurrence);
        };
        if request.occurrence_at_utc_ms < next_previewable {
            return Err(ScheduleSimulationError::NotOccurrence);
        }
        if cursor.structural_revision != request.expected_structural_revision
            || block.active_logic_revision() != request.expected_logic_revision
        {
            return Err(ScheduleSimulationError::StaleStructuralRevision);
        }
        let trigger = ScheduleTrigger {
            block_id: request.block_id.clone(),
            name: request.schedule.clone(),
            kind: block_schedule.rule.kind(),
            scheduled_for_utc_ms: request.occurrence_at_utc_ms,
            detected_at_utc_ms: request.occurrence_at_utc_ms,
            coalesced_count: 0,
            structural_revision: request.expected_structural_revision,
        };
        let execution = block.engine.simulate_schedule_trigger(
            trigger,
            &self.site,
            Some(request.occurrence_at_utc_ms),
        );
        Ok(BlockExecution {
            block_id: request.block_id,
            execution,
        })
    }

    /// A per-schedule status view for every configured schedule in block and
    /// declaration order. `enabled` is the effective flag (schedule AND
    /// block). Disabled schedules are `Paused`; an invalid wall clock reports
    /// `ClockError` for enabled schedules; schedules whose 370-day search
    /// found no occurrence report `Unavailable`.
    pub fn schedule_statuses(&self, utc_unix_ms: Option<i64>) -> Vec<ScheduleStatus> {
        let mut statuses = Vec::new();
        for block in &self.blocks {
            for block_schedule in &block.config.schedules {
                let enabled = block_schedule.enabled && block.enabled();
                let (status, next) = if !enabled {
                    (ScheduleStatusKind::Paused, None)
                } else if utc_unix_ms.is_none() {
                    (ScheduleStatusKind::ClockError, None)
                } else {
                    let key = (block.id().clone(), block_schedule.name.clone());
                    match self.schedule_cursors.get(&key) {
                        Some(cursor) => match cursor.next_occurrence_utc_ms {
                            Some(next) => (ScheduleStatusKind::Active, Some(next)),
                            None => (
                                ScheduleStatusKind::Unavailable {
                                    reason:
                                        "no occurrence found within the 370-day search window (polar day/night)"
                                            .to_owned(),
                                },
                                None,
                            ),
                        },
                        None => (ScheduleStatusKind::Paused, None),
                    }
                };
                statuses.push(ScheduleStatus {
                    block_id: block.id().clone(),
                    name: block_schedule.name.clone(),
                    enabled,
                    status,
                    next_occurrence_utc_ms: next,
                });
            }
        }
        statuses
    }

    pub fn simulate_input(
        &self,
        block_id: &BlockId,
        scenario: SimulationScenario,
    ) -> Result<BlockExecution, RuntimeSimulationError> {
        let block = self
            .block(block_id)
            .ok_or_else(|| RuntimeSimulationError::UnknownBlock(block_id.clone()))?;
        let execution = block.engine.simulate_input(scenario).map_err(|error| {
            RuntimeSimulationError::Block {
                block_id: block_id.clone(),
                error,
            }
        })?;
        Ok(BlockExecution {
            block_id: block_id.clone(),
            execution,
        })
    }

    pub fn simulate_input_with_state(
        &self,
        block_id: &BlockId,
        scenario: SimulationScenario,
        state: TransientState,
        pending_timers: Vec<PendingTimer>,
        now: MonotonicMs,
    ) -> Result<BlockExecution, RuntimeSimulationError> {
        let block = self
            .block(block_id)
            .ok_or_else(|| RuntimeSimulationError::UnknownBlock(block_id.clone()))?;
        let execution = block
            .engine
            .simulate_input_with_state(scenario, state, pending_timers, now)
            .map_err(|error| RuntimeSimulationError::Block {
                block_id: block_id.clone(),
                error,
            })?;
        Ok(BlockExecution {
            block_id: block_id.clone(),
            execution,
        })
    }

    pub fn simulate_timer(
        &self,
        block_id: &BlockId,
        scenario: TimerSimulationScenario,
    ) -> Result<BlockExecution, RuntimeSimulationError> {
        let block = self
            .block(block_id)
            .ok_or_else(|| RuntimeSimulationError::UnknownBlock(block_id.clone()))?;
        let execution = block.engine.simulate_timer(scenario).map_err(|error| {
            RuntimeSimulationError::Block {
                block_id: block_id.clone(),
                error,
            }
        })?;
        Ok(BlockExecution {
            block_id: block_id.clone(),
            execution,
        })
    }

    /// Validates every source in the batch before changing any active block.
    pub fn activate(
        &mut self,
        candidate: RuntimeActivation,
    ) -> Result<ActivationResult, ActivationError> {
        let mut indexes = Vec::with_capacity(candidate.blocks.len());
        let mut programs = Vec::with_capacity(candidate.blocks.len());
        for update in &candidate.blocks {
            if update.source.is_none() && update.enabled.is_none() {
                return Err(ActivationError::EmptyUpdate(update.block_id.clone()));
            }
            let index = self
                .blocks
                .iter()
                .position(|block| block.id() == &update.block_id)
                .ok_or_else(|| ActivationError::UnknownBlock(update.block_id.clone()))?;
            if indexes.contains(&index) {
                return Err(ActivationError::DuplicateBlock(update.block_id.clone()));
            }
            indexes.push(index);
            programs.push(
                update
                    .source
                    .as_deref()
                    .map(LogicProgram::try_new)
                    .transpose()
                    .map_err(|error| ActivationError::InvalidSource {
                        block_id: update.block_id.clone(),
                        error,
                    })?,
            );
        }

        let mut results = Vec::with_capacity(candidate.blocks.len());
        for ((update, index), program) in candidate.blocks.into_iter().zip(indexes).zip(programs) {
            let block = &mut self.blocks[index];
            let mut cancelled_timers = Vec::new();
            let source_changed = if let Some(program) = program {
                if program.revision == block.active_logic_revision() {
                    false
                } else {
                    cancelled_timers = block.engine.pending_timers.keys().cloned().collect();
                    block.engine.config.logic = program.clone();
                    block.engine.pending_timers.clear();
                    block.config.logic = program;
                    true
                }
            } else {
                false
            };
            let enabled_changed = update
                .enabled
                .is_some_and(|enabled| enabled != block.config.enabled);
            if enabled_changed {
                if !update.enabled.unwrap_or(block.config.enabled) {
                    cancelled_timers.extend(block.engine.pending_timers.keys().cloned());
                    block.engine.pending_timers.clear();
                }
                block.config.enabled = update.enabled.expect("enabled_changed implies value");
                // A block re-enable must establish a new future-only
                // baseline. Marking the cursors here keeps activation free of
                // host clock access; the desktop can immediately call
                // `rebaseline_block_schedules` with its paired sample, while
                // the next valid poll remains a safe fallback.
                for ((cursor_block_id, _), cursor) in self.schedule_cursors.iter_mut() {
                    if cursor_block_id == block.id() {
                        cursor.next_occurrence_utc_ms = None;
                        cursor.needs_rebaseline = true;
                    }
                }
            }
            cancelled_timers.sort();
            cancelled_timers.dedup();
            results.push(BlockActivationResult {
                block_id: block.id().clone(),
                logic_revision: block.active_logic_revision(),
                enabled: block.enabled(),
                source_changed,
                enabled_changed,
                cancelled_timers,
            });
        }
        Ok(ActivationResult { blocks: results })
    }

    fn block_index(&self, id: &BlockId) -> Result<usize, RuntimeEventError> {
        self.blocks
            .iter()
            .position(|block| block.id() == id)
            .ok_or_else(|| RuntimeEventError::UnknownBlock(id.clone()))
    }

    fn ensure_time(
        &self,
        block_id: Option<&BlockId>,
        now: MonotonicMs,
    ) -> Result<(), RuntimeEventError> {
        if let Some(previous) = self.last_accepted_at
            && now < previous
        {
            return Err(RuntimeEventError::TimeWentBackwards {
                block_id: block_id.cloned(),
                previous,
                current: now,
            });
        }
        Ok(())
    }

    fn ensure_schedule_time(&self, now: MonotonicMs) -> Result<(), TimeError> {
        if let Some(previous) = self.last_accepted_at
            && now < previous
        {
            return Err(TimeError::MonotonicWentBackwards {
                previous,
                current: now,
            });
        }
        Ok(())
    }

    /// Recomputes every schedule cursor strictly after `now_utc`, retaining a
    /// previously delivered occurrence as a lower bound. The latter is what
    /// prevents a backward wall-clock correction from replaying an occurrence
    /// that was already delivered before the correction.
    fn recompute_schedule_cursors(&mut self, now_utc: i64) {
        let structural_revision = self.schedule_structural_revision.unwrap_or(0);
        let schedules: Vec<(BlockId, ScheduleName, ScheduleRule)> = self
            .blocks
            .iter()
            .flat_map(|block| {
                block.config.schedules.iter().map(|block_schedule| {
                    (
                        block.id().clone(),
                        block_schedule.name.clone(),
                        block_schedule.rule.clone(),
                    )
                })
            })
            .collect();
        for (block_id, name, rule) in schedules {
            let key = (block_id, name);
            let cursor =
                self.schedule_cursors
                    .entry(key)
                    .or_insert_with(|| schedule::ScheduleCursor {
                        last_delivered_utc_ms: None,
                        next_occurrence_utc_ms: None,
                        structural_revision,
                        needs_rebaseline: true,
                    });
            cursor.structural_revision = structural_revision;
            cursor.next_occurrence_utc_ms = next_occurrence_after_not_before(
                &rule,
                &self.site,
                now_utc,
                cursor.last_delivered_utc_ms,
            );
            cursor.needs_rebaseline = false;
        }
    }
}
