impl DiagnosticStore {
pub fn record_telegram(&self, mut telegram: TelegramRecord) {
    let mut inner = self
        .inner
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if telegram.endpoint.is_none() {
        let address = telegram.address;
        telegram.endpoint = inner
            .endpoint_values
            .iter()
            .find(|(_, state)| state.address == Some(address))
            .map(|(name, _)| name.to_string());
    }
    if let (Some(endpoint), Some(value)) =
        (telegram.endpoint.as_deref(), telegram.value.as_ref())
        && let Ok(endpoint) = endpoint.parse::<EndpointName>()
        && let Some(state) = inner.endpoint_values.get_mut(&endpoint)
    {
        state.observed = Some(value.clone());
    }
    if let Some(value) = telegram.value.as_ref() {
        let address = telegram.address;
        for state in inner.block_endpoint_values.values_mut() {
            if state.address == Some(address) && state.direction == EndpointDirection::Input {
                state.observed = Some(value.clone());
            }
        }
    }
    inner.telegrams.push_back(telegram);
    while inner.telegrams.len() > inner.limits.recent_telegrams {
        inner.telegrams.pop_front();
    }
    self.publish_locked(&mut inner);
}

pub fn record_write_requested(
    &self,
    request_id: u64,
    block_id: &logiksmith_core::BlockId,
    endpoint: EndpointName,
    destination: GroupAddress,
    dpt: Dpt,
    value: TypedValue,
) {
    let mut inner = self
        .inner
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let value = ValueMessage::from_core(value);
    let execution_id = inner
        .blocks
        .get(block_id.as_str())
        .and_then(|block| block.last_result.as_ref())
        .map(|result| result.execution_id);
    if let Some(block_state) = inner
        .block_endpoint_values
        .get_mut(&(block_id.to_string(), endpoint.clone()))
    {
        block_state.requested = Some(value.clone());
    }
    if let Some(state) = inner.endpoint_values.get_mut(&endpoint) {
        state.requested = Some(value.clone());
        if state.address == Some(destination) && state.direction == EndpointDirection::Output {
            inner.last_write = WriteSnapshot {
                status: WriteStatus::Pending,
                request_id: Some(request_id),
                block_id: Some(block_id.to_string()),
                execution_id,
                value: Some(value.clone()),
                error: None,
            };
        }
    }
    let _ = dpt;
    if inner.pending_writes.len() >= inner.limits.pending_knx_writes
        && let Some(oldest) = inner.pending_writes.keys().next().copied()
    {
        inner.pending_writes.remove(&oldest);
    }
    inner.pending_writes.insert(request_id, WriteState);
    self.publish_locked(&mut inner);
}

pub fn record_write_result(&self, request_id: u64, ok: bool, error: Option<String>) {
    let mut inner = self
        .inner
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if inner.pending_writes.remove(&request_id).is_some()
        && inner.last_write.request_id == Some(request_id)
    {
        inner.last_write.status = if ok {
            WriteStatus::Succeeded
        } else {
            WriteStatus::Failed
        };
        inner.last_write.error = if ok { None } else { error };
    }
    self.publish_locked(&mut inner);
}
}
