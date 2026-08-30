impl Runtime {
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
                    .map(|source| LogicProgram::try_new_with_limits(source, self.limits))
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
                    block.reset_health();
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
                block.set_enabled(update.enabled.expect("enabled_changed implies value"));
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
}
