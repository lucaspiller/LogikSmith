fn process_runtime_operation(
    runtime: *mut RuntimeHandle,
    effects: *mut EffectRepr,
    capacity: usize,
    written: *mut usize,
    operation: impl FnOnce(
        &mut Runtime,
    ) -> Result<Vec<logiksmith_core::BlockExecution>, RuntimeEventError>,
) -> i32 {
    if written.is_null() {
        return Status::NullPointer.code();
    }
    unsafe { *written = 0 };
    if runtime.is_null() || (capacity > 0 && effects.is_null()) {
        return Status::NullPointer.code();
    }
    if capacity > MAX_EFFECTS {
        return Status::InvalidLength.code();
    }
    ffi_status(|| {
        let handle = unsafe { &mut *runtime };
        let before = handle.runtime.clone();
        let executions = match operation(&mut handle.runtime) {
            Ok(executions) => executions,
            Err(error) => {
                handle.runtime = before;
                return Err(status_for_runtime_event(&error));
            }
        };
        write_execution_effects(&executions, effects, capacity, written).map_err(|status| {
            handle.runtime = before;
            status
        })
    })
}

fn write_execution_effects(
    executions: &[logiksmith_core::BlockExecution],
    effects: *mut EffectRepr,
    capacity: usize,
    written: *mut usize,
) -> Result<Status, Status> {
    let output_count = count_effects(executions)?;
    if output_count > MAX_EFFECTS {
        return Err(Status::RuntimeLimit);
    }
    if output_count > capacity {
        // SAFETY: caller checked this pointer before entering the operation;
        // zero-capacity requests may use null and never write records.
        unsafe { *written = output_count };
        return Err(Status::OutputBufferTooSmall);
    }
    if output_count > 0 {
        // SAFETY: capacity is at least output_count and the caller supplied
        // writable storage for all records.
        let destination = unsafe { slice::from_raw_parts_mut(effects, output_count) };
        write_effects(executions, destination)?;
    }
    // SAFETY: written is non-null by every public caller.
    unsafe { *written = output_count };
    Ok(Status::Ok)
}

fn check_revision(current: u64, expected: u64) -> Result<(), Status> {
    if expected != 0 && expected != current {
        return Err(Status::RevisionConflict);
    }
    Ok(())
}

fn simulation_input_from_snapshot(input: &InputSnapshot) -> SimulationInput {
    SimulationInput {
        endpoint: input.endpoint.clone(),
        value: input.value,
        valid: input.valid,
        age_ms: input.age_ms,
    }
}

fn input_repr(input: &InputSnapshot) -> Result<InputRepr, Status> {
    let mut result = InputRepr::default();
    result.endpoint_len = copy_identifier_len(&mut result.endpoint, input.endpoint.as_str())?;
    result.valid = u8::from(input.valid);
    result.value = input
        .value
        .map(encode_value)
        .transpose()?
        .unwrap_or_default();
    result.age_ms = input.age_ms.unwrap_or(0);
    Ok(result)
}

fn timer_repr(timer: &logiksmith_core::PendingTimer) -> Result<TimerRepr, Status> {
    let mut result = TimerRepr::default();
    result.name_len = copy_identifier_len(&mut result.name, timer.name.as_str())?;
    result.scheduled_at = timer.scheduled_at.0;
    result.due_at = timer.due_at.0;
    result.scheduled_logic_revision = timer.scheduled_logic_revision;
    Ok(result)
}

fn copy_identifier_len(
    destination: &mut [u8; MAX_IDENTIFIER_BYTES],
    value: &str,
) -> Result<u16, Status> {
    if value.len() > MAX_IDENTIFIER_BYTES {
        return Err(Status::InvalidLength);
    }
    destination[..value.len()].copy_from_slice(value.as_bytes());
    destination[value.len()..].fill(0);
    Ok(value.len() as u16)
}

fn health_code(health: logiksmith_core::BlockHealth) -> u8 {
    match health {
        logiksmith_core::BlockHealth::Disabled => 0,
        logiksmith_core::BlockHealth::Active => 1,
        logiksmith_core::BlockHealth::SuspendedScriptFailures => 2,
        logiksmith_core::BlockHealth::SuspendedEventRate => 3,
    }
}

fn ffi_status(operation: impl FnOnce() -> Result<Status, Status>) -> i32 {
    match catch_unwind(AssertUnwindSafe(operation)) {
        Ok(Ok(status)) | Ok(Err(status)) => status.code(),
        Err(_) => Status::Panic.code(),
    }
}

fn read_array<'a, T>(pointer: *const T, length: usize, maximum: usize) -> Result<&'a [T], Status> {
    if length > maximum {
        return Err(Status::InvalidLength);
    }
    if length == 0 {
        return Ok(&[]);
    }
    if pointer.is_null() {
        return Err(Status::NullPointer);
    }
    // SAFETY: caller contract supplies a readable array of `length` entries.
    Ok(unsafe { slice::from_raw_parts(pointer, length) })
}

fn with_bytes<T>(
    pointer: *const u8,
    length: usize,
    maximum: usize,
    operation: impl FnOnce(&[u8]) -> Result<T, Status>,
) -> Result<T, Status> {
    if length > maximum {
        return Err(Status::InvalidLength);
    }
    if length == 0 {
        return operation(&[]);
    }
    if pointer.is_null() {
        return Err(Status::NullPointer);
    }
    // SAFETY: caller contract supplies a readable byte string of `length`.
    operation(unsafe { slice::from_raw_parts(pointer, length) })
}

fn with_utf8<T>(
    pointer: *const u8,
    length: usize,
    maximum: usize,
    operation: impl FnOnce(&str) -> Result<T, Status>,
) -> Result<T, Status> {
    with_bytes(pointer, length, maximum, |bytes| {
        let value = str::from_utf8(bytes).map_err(|_| Status::InvalidUtf8)?;
        operation(value)
    })
}

fn decode_block_id(pointer: *const u8, length: usize) -> Result<BlockId, Status> {
    with_utf8(pointer, length, MAX_IDENTIFIER_BYTES, |value| {
        value.parse().map_err(|_| Status::InvalidBlockId)
    })
}

fn decode_endpoint_name(pointer: *const u8, length: usize) -> Result<EndpointName, Status> {
    with_utf8(pointer, length, MAX_IDENTIFIER_BYTES, |value| {
        value.parse().map_err(|_| Status::InvalidEndpointName)
    })
}

fn decode_block_config(config: &BlockConfigRepr) -> Result<BlockConfig, Status> {
    let block_id = decode_block_id(config.block_id, config.block_id_len)?;
    let source = with_utf8(
        config.logic_source,
        config.logic_source_len,
        MAX_SOURCE_BYTES,
        |value| Ok(value.to_owned()),
    )?;
    let endpoint_configs = read_array(
        config.endpoints,
        config.endpoint_count,
        MAX_ENDPOINTS_PER_BLOCK,
    )?;
    if endpoint_configs.is_empty() {
        return Err(Status::InvalidLength);
    }
    let mut endpoints = Vec::with_capacity(endpoint_configs.len());
    for endpoint in endpoint_configs {
        if endpoint.reserved != [0; 3] {
            return Err(Status::InvalidConfiguration);
        }
        let name = decode_endpoint_name(endpoint.name, endpoint.name_len)?;
        let direction = match endpoint.direction {
            INPUT => EndpointDirection::Input,
            OUTPUT => EndpointDirection::Output,
            _ => return Err(Status::InvalidConfiguration),
        };
        let dpt =
            Dpt::new(endpoint.dpt_major, endpoint.dpt_subtype).map_err(|_| Status::InvalidDpt)?;
        if !dpt.is_supported() {
            return Err(Status::InvalidDpt);
        }
        endpoints.push(Endpoint::new(name, direction, dpt));
    }
    Ok(BlockConfig::new(block_id, true, endpoints, source))
}

fn decode_value(value: ValueRepr) -> Result<TypedValue, Status> {
    if value.reserved != 0 || value.reserved2 != 0 {
        return Err(Status::InvalidValue);
    }
    let dpt = Dpt::new(value.dpt_major, value.dpt_subtype).map_err(|_| Status::InvalidDpt)?;
    let semantic = match value.kind {
        VALUE_BOOL if value.scalar == 0 || value.scalar == 1 => Value::Bool(value.scalar == 1),
        VALUE_PERCENT if (0..=100).contains(&value.scalar) => Value::Percent(value.scalar as u8),
        VALUE_TEMPERATURE_CENTI_DEGREES => Value::Temperature(value.scalar),
        _ => return Err(Status::InvalidValue),
    };
    TypedValue::new(dpt, semantic).map_err(|_| Status::InvalidValue)
}

fn count_effects(executions: &[logiksmith_core::BlockExecution]) -> Result<usize, Status> {
    let mut count = 0usize;
    for execution in executions {
        let transition = execution
            .execution
            .outcome
            .as_ref()
            .map_err(|_| Status::LogicError)?;
        count = count
            .checked_add(transition.outputs.len())
            .ok_or(Status::InvalidLength)?;
    }
    Ok(count)
}

fn write_effects(
    executions: &[logiksmith_core::BlockExecution],
    destination: &mut [EffectRepr],
) -> Result<(), Status> {
    let mut index = 0;
    for execution in executions {
        let transition = execution
            .execution
            .outcome
            .as_ref()
            .map_err(|_| Status::LogicError)?;
        for output in &transition.outputs {
            let record = destination.get_mut(index).ok_or(Status::InvalidLength)?;
            copy_identifier(
                &mut record.block_id,
                &mut record.block_id_len,
                execution.block_id.as_str(),
            )?;
            copy_identifier(
                &mut record.endpoint,
                &mut record.endpoint_len,
                output.endpoint.as_str(),
            )?;
            record.value = encode_value(output.value)?;
            index += 1;
        }
    }
    Ok(())
}

fn copy_identifier(
    destination: &mut [u8; MAX_IDENTIFIER_BYTES],
    length: &mut u16,
    value: &str,
) -> Result<(), Status> {
    if value.len() > MAX_IDENTIFIER_BYTES {
        return Err(Status::InvalidLength);
    }
    destination[..value.len()].copy_from_slice(value.as_bytes());
    destination[value.len()..].fill(0);
    *length = value.len() as u16;
    Ok(())
}

fn encode_value(value: TypedValue) -> Result<ValueRepr, Status> {
    let (kind, scalar) = match value.value() {
        Value::Bool(value) => (VALUE_BOOL, i32::from(value)),
        Value::Percent(value) => (VALUE_PERCENT, i32::from(value)),
        Value::Temperature(value) => (VALUE_TEMPERATURE_CENTI_DEGREES, value),
    };
    Ok(ValueRepr {
        dpt_major: value.dpt().major(),
        dpt_subtype: value.dpt().subtype(),
        kind,
        reserved: 0,
        reserved2: 0,
        scalar,
    })
}

fn status_for_runtime_event(error: &RuntimeEventError) -> Status {
    match error {
        RuntimeEventError::UnknownBlock(_) => Status::UnknownBlock,
        RuntimeEventError::TimeWentBackwards { .. } => Status::TimeWentBackwards,
        RuntimeEventError::CascadeLimit { .. }
        | RuntimeEventError::ResourceLimit { .. }
        | RuntimeEventError::CascadeTimeLimit { .. } => Status::RuntimeLimit,
        RuntimeEventError::Block { error, .. } => match error {
            logiksmith_core::EventError::UnknownEndpoint(_) => Status::UnknownEndpoint,
            logiksmith_core::EventError::EndpointNotInput { .. } => Status::EndpointNotInput,
            logiksmith_core::EventError::DptMismatch { .. } => Status::DptMismatch,
            logiksmith_core::EventError::InvalidValue(_) => Status::InvalidValue,
            logiksmith_core::EventError::TimeWentBackwards { .. } => Status::TimeWentBackwards,
            logiksmith_core::EventError::StaleTimer { .. } => Status::InvalidConfiguration,
        },
    }
}
