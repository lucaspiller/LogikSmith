impl Runtime {
    /// Applies one transport-neutral update to a block input and propagates
    /// any resulting signal effects.
    pub fn process_input_update(
        &mut self,
        block_id: &BlockId,
        endpoint: EndpointName,
        update: InputUpdate,
        now: MonotonicMs,
    ) -> Result<Vec<BlockExecution>, RuntimeEventError> {
        self.process_input_update_sampled(
            block_id,
            endpoint,
            update,
            ClockSample { monotonic_ms: now, utc_unix_ms: None },
        )
    }
    /// ClockSample variant for host-owned transport updates. Observe and
    /// invalidate updates only change the target snapshot; triggers evaluate
    /// the enabled block and run its existing signal cascade.
    pub fn process_input_update_sampled(
        &mut self,
        block_id: &BlockId,
        endpoint: EndpointName,
        update: InputUpdate,
        sample: ClockSample,
    ) -> Result<Vec<BlockExecution>, RuntimeEventError> {
        let Some(root) = self.process_input_update_sampled_single(block_id, endpoint, update, sample)? else {
            return Ok(Vec::new());
        };
        let mut executions = vec![root];
        self.propagate_from_execution(0, sample.monotonic_ms, sample.utc_unix_ms, &mut executions)?;
        Ok(executions)
    }

    fn process_input_update_sampled_single(
        &mut self,
        block_id: &BlockId,
        endpoint: EndpointName,
        update: InputUpdate,
        sample: ClockSample,
    ) -> Result<Option<BlockExecution>, RuntimeEventError> {
        let index = self.block_index(block_id)?;
        match update {
            InputUpdate::Observe(value) | InputUpdate::Trigger(value) => self.blocks[index]
                .engine
                .validate_input(&endpoint, value)
                .map_err(|error| RuntimeEventError::Block { block_id: block_id.clone(), error })?,
            InputUpdate::Invalidate => self.blocks[index]
                .engine
                .validate_endpoint(&endpoint)
                .map_err(|error| RuntimeEventError::Block { block_id: block_id.clone(), error })?,
        };
        self.ensure_time(Some(block_id), sample.monotonic_ms)?;
        let update = if !self.blocks[index].config.enabled {
            match update {
                InputUpdate::Trigger(value) => InputUpdate::Observe(value),
                update => update,
            }
        } else {
            update
        };
        let execution = self.blocks[index]
            .engine
            .process_input_update_sampled(endpoint, update, sample, &self.site)
            .map_err(|error| RuntimeEventError::Block { block_id: block_id.clone(), error })?;
        self.last_accepted_at = Some(sample.monotonic_ms);
        let Some(mut execution) = execution else { return Ok(None); };
        self.assign_execution_id(&mut execution, None);
        Ok(Some(BlockExecution { block_id: block_id.clone(), execution }))
    }
}
