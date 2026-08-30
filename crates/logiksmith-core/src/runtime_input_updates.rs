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
        self.process_input_update_sampled_with_probe(block_id, endpoint, update, sample, None)
    }

    /// ClockSample variant which threads a host-owned elapsed-time probe into
    /// each live Lua handler reached by the update and its signal cascade.
    pub fn process_input_update_sampled_with_budget_probe(
        &mut self,
        block_id: &BlockId,
        endpoint: EndpointName,
        update: InputUpdate,
        sample: ClockSample,
        budget_probe: BudgetProbeHandle,
    ) -> Result<Vec<BlockExecution>, RuntimeEventError> {
        self.process_input_update_sampled_with_probe(
            block_id,
            endpoint,
            update,
            sample,
            Some(budget_probe),
        )
    }

    fn process_input_update_sampled_with_probe(
        &mut self,
        block_id: &BlockId,
        endpoint: EndpointName,
        update: InputUpdate,
        sample: ClockSample,
        budget_probe: Option<BudgetProbeHandle>,
    ) -> Result<Vec<BlockExecution>, RuntimeEventError> {
        let checkpoint = self.clone();
        let root = match self.process_input_update_sampled_single(
            block_id,
            endpoint,
            update,
            sample,
            budget_probe.clone(),
        ) {
            Ok(root) => root,
            Err(error) => {
                *self = checkpoint;
                return Err(error);
            }
        };
        let Some(root) = root else {
            return Ok(Vec::new());
        };
        let mut executions = vec![root];
        if let Err(error) = self.propagate_from_execution(
            0,
            sample.monotonic_ms,
            sample.utc_unix_ms,
            budget_probe,
            &mut executions,
        ) {
            if should_rollback_after_propagation(&error) {
                *self = checkpoint;
            }
            return Err(error);
        }
        if let Err(error) = self.validate_usage() {
            *self = checkpoint;
            return Err(error);
        }
        Ok(executions)
    }

    fn process_input_update_sampled_single(
        &mut self,
        block_id: &BlockId,
        endpoint: EndpointName,
        update: InputUpdate,
        sample: ClockSample,
        budget_probe: Option<BudgetProbeHandle>,
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
        let update = if !self.blocks[index].config.enabled
            || self.blocks[index].health() != BlockHealth::Active
        {
            match update {
                InputUpdate::Trigger(value) => InputUpdate::Observe(value),
                update => update,
            }
        } else {
            update
        };
        let update = match update {
            InputUpdate::Trigger(value)
                if !self.blocks[index].admit_live_execution(
                    sample.monotonic_ms,
                    self.limits.max_live_executions_per_block_per_second,
                ) => InputUpdate::Observe(value),
            update => update,
        };
        let live_execution = matches!(update, InputUpdate::Trigger(_));
        let execution = match budget_probe {
            Some(budget_probe) => self.blocks[index]
                .engine
                .process_input_update_sampled_with_budget_probe(
                    endpoint,
                    update,
                    sample,
                    &self.site,
                    budget_probe,
                ),
            None => self.blocks[index]
                .engine
                .process_input_update_sampled(endpoint, update, sample, &self.site),
        }
            .map_err(|error| RuntimeEventError::Block { block_id: block_id.clone(), error })?;
        self.last_accepted_at = Some(sample.monotonic_ms);
        let Some(mut execution) = execution else { return Ok(None); };
        if live_execution {
            self.blocks[index].record_live_execution(&execution);
        }
        self.assign_execution_id(&mut execution, None);
        Ok(Some(BlockExecution { block_id: block_id.clone(), execution }))
    }
}
