impl Runtime {
    /// Validates a schedule simulation selection and evaluates it without
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
        let mut execution = block.engine.simulate_schedule_trigger(
            trigger,
            &self.site,
            Some(request.occurrence_at_utc_ms),
        );
        self.annotate_simulation(&request.block_id, &mut execution);
        Ok(BlockExecution {
            block_id: request.block_id,
            execution,
        })
    }

    pub fn simulate_input(
        &self,
        block_id: &BlockId,
        scenario: SimulationScenario,
    ) -> Result<BlockExecution, RuntimeSimulationError> {
        let block = self
            .block(block_id)
            .ok_or_else(|| RuntimeSimulationError::UnknownBlock(block_id.clone()))?;
        let mut execution = block.engine.simulate_input(scenario).map_err(|error| {
            RuntimeSimulationError::Block {
                block_id: block_id.clone(),
                error,
            }
        })?;
        self.annotate_simulation(block_id, &mut execution);
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
        let mut execution = block
            .engine
            .simulate_input_with_state(scenario, state, pending_timers, now)
            .map_err(|error| RuntimeSimulationError::Block {
                block_id: block_id.clone(),
                error,
            })?;
        self.annotate_simulation(block_id, &mut execution);
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
        let mut execution = block.engine.simulate_timer(scenario).map_err(|error| {
            RuntimeSimulationError::Block {
                block_id: block_id.clone(),
                error,
            }
        })?;
        self.annotate_simulation(block_id, &mut execution);
        Ok(BlockExecution {
            block_id: block_id.clone(),
            execution,
        })
    }
}
