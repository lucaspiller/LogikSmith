impl Runtime {
    pub fn persistent_state_enabled(&self) -> bool {
        self.persistent_state_enabled
    }

    /// Replaces the complete host-loaded persistent snapshot before events
    /// are accepted. Unknown block IDs are rejected so hosts can prune them
    /// deliberately before calling this method.
    pub fn load_persistent_state_snapshot(
        &mut self,
        states: BTreeMap<BlockId, PersistentState>,
    ) -> Result<(), PersistentStateError> {
        if !self.persistent_state_enabled && states.values().any(|state| !state.is_empty()) {
            return Err(PersistentStateError::Disabled);
        }
        for block_id in states.keys() {
            if !self.blocks.iter().any(|block| block.id() == block_id) {
                return Err(PersistentStateError::UnknownBlock(block_id.clone()));
            }
        }
        let mut runtime_bytes = 0usize;
        for block in &self.blocks {
            let persistent = states.get(block.id()).cloned().unwrap_or_default();
            validate_combined_state_with_limits(
                block.engine.state(),
                &persistent,
                &self.limits,
            )
            .map_err(|error| PersistentStateError::InvalidBlockState {
                block_id: block.id().clone(),
                error,
            })?;
            runtime_bytes = runtime_bytes
                .saturating_add(state_bytes(block.engine.state()))
                .saturating_add(state_bytes(&persistent));
        }
        if runtime_bytes > self.limits.max_state_bytes_total {
            return Err(PersistentStateError::RuntimeStateTooLarge {
                actual: runtime_bytes,
                maximum: self.limits.max_state_bytes_total,
            });
        }
        for block in &mut self.blocks {
            let persistent = states.get(block.id()).cloned().unwrap_or_default();
            block.engine.load_persistent_state(persistent).map_err(|error| {
                PersistentStateError::InvalidBlockState {
                    block_id: block.id().clone(),
                    error,
                }
            })?;
        }
        Ok(())
    }

    /// Captures the complete persistent snapshot in stable block-ID order.
    pub fn persistent_state_snapshot(&self) -> BTreeMap<BlockId, PersistentState> {
        self.blocks
            .iter()
            .map(|block| (block.id().clone(), block.engine.persistent_state().clone()))
            .collect()
    }
}
