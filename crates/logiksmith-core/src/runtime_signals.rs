impl Runtime {
    /// Returns signal snapshots using the runtime's most recently accepted
    /// monotonic time.
    pub fn signal_snapshots(&self) -> Vec<SignalSnapshot> {
        self.signal_snapshots_at(self.last_accepted_at.unwrap_or_default())
    }

    pub fn signal_snapshots_at(&self, _now: MonotonicMs) -> Vec<SignalSnapshot> {
        self.signals
            .iter()
            .map(|signal| {
                let status = match signal.value {
                    None => SignalStatus::Unknown,
                    Some(_) if signal.producer.as_ref().is_some_and(|producer| {
                        self.block(&producer.block_id)
                            .is_some_and(|block| !block.enabled())
                    }) => SignalStatus::ProducerDisabled,
                    Some(_) => SignalStatus::Valid,
                };
                SignalSnapshot {
                    name: signal.config.name.clone(),
                    dpt: signal.config.dpt,
                    value: signal.value,
                    status,
                    observed_at: signal.observed_at,
                    changed_at: signal.changed_at,
                    producer: signal.producer.clone(),
                    producing_execution: signal.producing_execution,
                    consumers: signal.consumers.clone(),
                }
            })
            .collect()
    }

    pub fn signal_snapshot(&self, name: &SignalName) -> Option<SignalSnapshot> {
        self.signal_snapshots()
            .into_iter()
            .find(|signal| signal.name == *name)
    }

    pub fn signal_config(&self, name: &SignalName) -> Option<&SignalConfig> {
        self.signals
            .iter()
            .find(|signal| signal.config.name == *name)
            .map(|signal| &signal.config)
    }

    pub fn process_input_cascade(
        &mut self,
        block_id: &BlockId,
        event: InputEvent,
        now: MonotonicMs,
    ) -> Result<Vec<BlockExecution>, RuntimeEventError> {
        let Some(root) = self.process_input(block_id, event, now)? else {
            return Ok(Vec::new());
        };
        let mut executions = vec![root];
        self.propagate_from_execution(0, now, None, &mut executions)?;
        Ok(executions)
    }

    pub fn process_input_cascade_sampled(
        &mut self,
        block_id: &BlockId,
        event: InputEvent,
        sample: ClockSample,
    ) -> Result<Vec<BlockExecution>, RuntimeEventError> {
        let Some(root) = self.process_input_sampled(block_id, event, sample)? else {
            return Ok(Vec::new());
        };
        let mut executions = vec![root];
        self.propagate_from_execution(
            0,
            sample.monotonic_ms,
            sample.utc_unix_ms,
            &mut executions,
        )?;
        Ok(executions)
    }

    pub fn process_next_due_timer_cascade(
        &mut self,
        now: MonotonicMs,
    ) -> Result<Vec<BlockExecution>, RuntimeEventError> {
        let Some(root) = self.process_next_due_timer(now)? else {
            return Ok(Vec::new());
        };
        let mut executions = vec![root];
        self.propagate_from_execution(0, now, None, &mut executions)?;
        Ok(executions)
    }

    pub fn process_next_due_timer_cascade_sampled(
        &mut self,
        sample: ClockSample,
    ) -> Result<Vec<BlockExecution>, RuntimeEventError> {
        let Some(root) = self.process_next_due_timer_sampled(sample)? else {
            return Ok(Vec::new());
        };
        let mut executions = vec![root];
        self.propagate_from_execution(
            0,
            sample.monotonic_ms,
            sample.utc_unix_ms,
            &mut executions,
        )?;
        Ok(executions)
    }

    pub fn process_schedule_cascade(
        &mut self,
        trigger: ScheduleTrigger,
    ) -> Result<Vec<BlockExecution>, RuntimeEventError> {
        let Some(root) = self.process_schedule(trigger.clone())? else {
            return Ok(Vec::new());
        };
        let mut executions = vec![root];
        self.propagate_from_execution(
            0,
            self.last_accepted_at.unwrap_or_default(),
            Some(trigger.scheduled_for_utc_ms),
            &mut executions,
        )?;
        Ok(executions)
    }

    pub fn process_schedule_cascade_sampled(
        &mut self,
        trigger: ScheduleTrigger,
        sample: ClockSample,
    ) -> Result<Vec<BlockExecution>, RuntimeEventError> {
        let Some(root) = self.process_schedule_sampled(trigger, sample)? else {
            return Ok(Vec::new());
        };
        let mut executions = vec![root];
        self.propagate_from_execution(
            0,
            sample.monotonic_ms,
            sample.utc_unix_ms,
            &mut executions,
        )?;
        Ok(executions)
    }

    pub(crate) fn assign_execution_id(
        &mut self,
        execution: &mut Execution,
        causal_producer: Option<ExecutionId>,
    ) {
        let id = self.next_execution_id;
        self.next_execution_id = self.next_execution_id.saturating_add(1);
        execution.set_runtime_metadata(id, causal_producer);
    }

    fn propagate_from_execution(
        &mut self,
        execution_index: usize,
        now: MonotonicMs,
        utc_unix_ms: Option<i64>,
        executions: &mut Vec<BlockExecution>,
    ) -> Result<(), RuntimeEventError> {
        let block_id = executions[execution_index].block_id.clone();
        let execution_id = executions[execution_index].execution.id;
        let outputs = match &executions[execution_index].execution.outcome {
            Ok(transition) => transition.outputs.clone(),
            Err(_) => return Ok(()),
        };
        for output in outputs {
            let Some(signal_name) = self.blocks[self.block_index_unchecked(&block_id)]
                .config
                .signal_bindings
                .iter()
                .find(|binding| binding.endpoint == output.endpoint)
                .map(|binding| binding.signal.clone())
            else {
                continue;
            };
            let Some(signal_index) = self.signal_indexes.get(&signal_name).copied() else {
                continue;
            };
            let producer = SignalEndpointId::new(block_id.clone(), output.endpoint.clone());
            let (changed, consumers, effect) = {
                let signal = &mut self.signals[signal_index];
                let changed = signal.value != Some(output.value);
                signal.value = Some(output.value);
                signal.observed_at = Some(now);
                if changed {
                    signal.changed_at = Some(now);
                }
                signal.producing_execution = execution_id;
                let consumers = signal.consumers.clone();
                let effect = SignalEffect {
                    signal: signal_name,
                    value: output.value,
                    changed,
                    producer,
                    producing_execution: execution_id,
                    consumers: consumers.clone(),
                };
                (changed, consumers, effect)
            };
            let mut signal_effects = executions[execution_index]
                .execution
                .signal_effects
                .clone();
            signal_effects.push(effect);
            let mut eligible = executions[execution_index]
                .execution
                .eligible_consumers
                .clone();
            if changed {
                eligible.extend(
                    consumers
                        .iter()
                        .filter(|consumer| {
                            self.block(&consumer.block_id)
                                .is_some_and(|block| block.enabled())
                        })
                        .cloned(),
                );
            }
            executions[execution_index]
                .execution
                .set_signal_metadata(signal_effects, eligible);
            if !changed {
                // Equal values are not executions, but they are still fresh
                // observations for every consumer input snapshot.
                for consumer in consumers {
                    let Some(consumer_index) = self.blocks.iter().position(|block| {
                        block.id() == &consumer.block_id
                    }) else {
                        continue;
                    };
                    self.blocks[consumer_index]
                        .engine
                        .observe_input(InputObservation::new(consumer.endpoint.clone(), output.value), now)
                        .map_err(|error| RuntimeEventError::Block {
                            block_id: consumer.block_id,
                            error,
                        })?;
                }
                continue;
            }
            for consumer in consumers {
                let Some(consumer_index) = self.blocks.iter().position(|block| {
                    block.id() == &consumer.block_id
                }) else {
                    continue;
                };
                let input_name = self.blocks[consumer_index]
                    .config
                    .endpoints
                    .iter()
                    .find(|endpoint| endpoint.name == consumer.endpoint)
                    .expect("validated signal consumer endpoint")
                    .name
                    .clone();
                let event = InputEvent::new(input_name.clone(), output.value);
                if !self.blocks[consumer_index].config.enabled {
                    self.blocks[consumer_index]
                        .engine
                        .observe_input(InputObservation::new(input_name, output.value), now)
                        .map_err(|error| RuntimeEventError::Block {
                            block_id: consumer.block_id.clone(),
                            error,
                        })?;
                    continue;
                }
                let child = if utc_unix_ms.is_some() {
                    self.blocks[consumer_index]
                        .engine
                        .process_input_sampled(
                            event,
                            ClockSample {
                                monotonic_ms: now,
                                utc_unix_ms,
                            },
                            &self.site,
                        )
                } else {
                    self.blocks[consumer_index].engine.process_input(event, now)
                }
                .map_err(|error| RuntimeEventError::Block {
                    block_id: consumer.block_id.clone(),
                    error,
                })?;
                self.last_accepted_at = Some(now);
                let mut child = child;
                self.assign_execution_id(&mut child, execution_id);
                let child_index = executions.len();
                executions.push(BlockExecution {
                    block_id: consumer.block_id.clone(),
                    execution: child,
                });
                self.propagate_from_execution(child_index, now, utc_unix_ms, executions)?;
            }
        }
        Ok(())
    }

    fn block_index_unchecked(&self, id: &BlockId) -> usize {
        self.blocks
            .iter()
            .position(|block| block.id() == id)
            .expect("execution block is present in runtime")
    }

    fn annotate_simulation(&self, block_id: &BlockId, execution: &mut Execution) {
        let Some(block) = self.block(block_id) else {
            return;
        };
        let outputs = match &execution.outcome {
            Ok(transition) => &transition.outputs,
            Err(_) => return,
        };
        let mut effects = Vec::new();
        let mut eligible = Vec::new();
        for output in outputs {
            let Some(binding) = block
                .config
                .signal_bindings
                .iter()
                .find(|binding| binding.endpoint == output.endpoint)
            else {
                continue;
            };
            let Some(signal_index) = self.signal_indexes.get(&binding.signal).copied() else {
                continue;
            };
            let signal = &self.signals[signal_index];
            let producer = SignalEndpointId::new(block_id.clone(), output.endpoint.clone());
            let consumers = signal.consumers.clone();
            let changed = signal.value != Some(output.value);
            effects.push(SignalEffect {
                signal: binding.signal.clone(),
                value: output.value,
                changed,
                producer,
                producing_execution: None,
                consumers: consumers.clone(),
            });
            if changed {
                eligible.extend(
                    consumers
                        .into_iter()
                        .filter(|consumer| {
                            self.block(&consumer.block_id)
                                .is_some_and(|block| block.enabled())
                        }),
                );
            }
        }
        execution.set_signal_metadata(effects, eligible);
    }
}
