/// Return the ABI version expected by the C header.
#[unsafe(no_mangle)]
pub extern "C" fn logiksmith_abi_version() -> u32 {
    ABI_VERSION
}

/// Reports whether this embedded ABI build supports host-backed persistent state.
#[unsafe(no_mangle)]
pub extern "C" fn logiksmith_persistent_state_supported() -> u8 {
    0
}

/// Construct a runtime from bounded C arrays and UTF-8 byte strings.
///
/// # Safety
/// `config`, every non-empty array, and every non-empty byte string must point
/// to readable memory for the duration of the call. `out_runtime` must be a
/// writable pointer to one runtime handle slot. A successful handle is owned
/// by the caller and must be destroyed exactly once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn logiksmith_runtime_create(
    config: *const RuntimeConfigRepr,
    out_runtime: *mut *mut RuntimeHandle,
) -> i32 {
    if out_runtime.is_null() {
        return Status::NullPointer.code();
    }
    // SAFETY: checked above; initialize the caller's result before doing any
    // fallible work so every non-success path has a deterministic result.
    unsafe { *out_runtime = std::ptr::null_mut() };
    ffi_status(|| {
        // SAFETY: caller guarantees config points to a readable struct.
        let config = unsafe { config.as_ref() }.ok_or(Status::NullPointer)?;
        let blocks = read_array(config.blocks, config.block_count, MAX_BLOCKS)?;
        if blocks.is_empty() {
            return Err(Status::InvalidLength);
        }
        let mut block_configs = Vec::with_capacity(blocks.len());
        for block in blocks {
            block_configs.push(decode_block_config(block)?);
        }
        let runtime = Runtime::try_new_with_profile(
            RuntimeConfig::new(block_configs),
            logiksmith_core::RuntimeProfile::EmbeddedBaseline,
        )
        .map_err(|_| Status::InvalidConfiguration)?;
        let handle = Box::new(RuntimeHandle {
            runtime,
            document_revision: 1,
            structural_revision: 1,
        });
        // SAFETY: out_runtime is non-null and caller supplied writable storage.
        unsafe { *out_runtime = Box::into_raw(handle) };
        Ok(Status::Ok)
    })
}

/// Construct a runtime from a validated document/configuration wrapper.
///
/// The document's bytes and nested arrays are copied into the Rust runtime;
/// no C-owned allocation is retained after this call returns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn logiksmith_runtime_create_from_document(
    document: *const RuntimeDocumentRepr,
    out_runtime: *mut *mut RuntimeHandle,
) -> i32 {
    if document.is_null() {
        return Status::NullPointer.code();
    }
    // SAFETY: the caller supplies a readable document wrapper.
    let document = unsafe { &*document };
    if document.document_revision == 0 || document.structural_revision == 0 {
        return Status::InvalidConfiguration.code();
    }
    let status = unsafe { logiksmith_runtime_create(document.config, out_runtime) };
    if status != Status::Ok.code() {
        return status;
    }
    // SAFETY: create returned a valid handle in the caller's slot.
    unsafe {
        (*out_runtime)
            .as_mut()
            .expect("successful create handle")
            .document_revision = document.document_revision;
        (*out_runtime)
            .as_mut()
            .expect("successful create handle")
            .structural_revision = document.structural_revision;
    }
    Status::Ok.code()
}

/// Parse and validate one complete automation.toml document using the shared
/// embedded profile, then construct the ABI runtime from its core projection.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn logiksmith_runtime_create_from_toml(
    document: *const RuntimeDocumentBytesRepr,
    out_runtime: *mut *mut RuntimeHandle,
) -> i32 {
    if document.is_null() || out_runtime.is_null() {
        return Status::NullPointer.code();
    }
    unsafe { *out_runtime = std::ptr::null_mut() };
    ffi_status(|| {
        let document = unsafe { &*document };
        if document.document_revision == 0
            || document.structural_revision == 0
            || document.len > MAX_DOCUMENT_BYTES
        {
            return Err(Status::InvalidLength);
        }
        let parsed = with_bytes(document.bytes, document.len, MAX_DOCUMENT_BYTES, |bytes| {
            let source = str::from_utf8(bytes).map_err(|_| Status::InvalidUtf8)?;
            parse_automation_toml(source).map_err(|_| Status::InvalidConfiguration)
        })?;
        let api =
            RuntimeApi::from_document(parsed, logiksmith_core::RuntimeProfile::EmbeddedBaseline)
                .map_err(|_| Status::InvalidConfiguration)?;
        let handle = Box::new(RuntimeHandle {
            runtime: api.runtime().clone(),
            document_revision: document.document_revision,
            structural_revision: document.structural_revision,
        });
        unsafe { *out_runtime = Box::into_raw(handle) };
        Ok(Status::Ok)
    })
}

/// Destroy a handle previously returned by [`runtime_create`].
///
/// A null handle is accepted as an idempotent no-op. Any non-null pointer must
/// be the live pointer returned by this crate and must not be used again.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn logiksmith_runtime_destroy(runtime: *mut RuntimeHandle) -> i32 {
    if runtime.is_null() {
        return Status::Ok.code();
    }
    ffi_status(|| {
        // SAFETY: caller contract requires an owned pointer from create.
        unsafe { drop(Box::from_raw(runtime)) };
        Ok(Status::Ok)
    })
}

/// Process a triggering input and copy logical effects into a caller buffer.
///
/// The operation is transactional with respect to output capacity: when the
/// buffer is too small, the runtime is restored and `written` reports the
/// required number of records. The runtime may still accept an input whose Lua
/// program returns a logic error; in that case no effects are emitted and the
/// status is [`Status::LogicError`].
///
/// # Safety
/// `runtime`, `event`, and `written` must be valid pointers. `effects` must be
/// writable for `capacity` records, unless `capacity` is zero (where null is
/// accepted). Pointers inside `event` must remain readable for this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn logiksmith_runtime_process_input(
    runtime: *mut RuntimeHandle,
    event: *const InputEventRepr,
    effects: *mut EffectRepr,
    capacity: usize,
    written: *mut usize,
) -> i32 {
    if written.is_null() {
        return Status::NullPointer.code();
    }
    // SAFETY: checked above; all return paths below leave this initialized.
    unsafe { *written = 0 };
    if runtime.is_null() || event.is_null() {
        return Status::NullPointer.code();
    }
    if capacity > 0 && effects.is_null() {
        return Status::NullPointer.code();
    }
    if capacity > MAX_EFFECTS {
        return Status::InvalidLength.code();
    }
    ffi_status(|| {
        // SAFETY: pointers were checked and caller owns their readable data.
        let event = unsafe { event.as_ref() }.ok_or(Status::NullPointer)?;
        let block_id = decode_block_id(event.block_id, event.block_id_len)?;
        let endpoint = decode_endpoint_name(event.endpoint, event.endpoint_len)?;
        if event.group_address == 0 || event.group_address > MAX_KNX_GROUP_ADDRESS {
            return Err(Status::InvalidGroupAddress);
        }
        let value = decode_value(event.value)?;
        let input = InputEvent::new(endpoint, value);

        // Core events can assign input state and then traverse a signal
        // cascade. Keep a clone so a caller can retry after buffer sizing or a
        // transport-facing routing error without a partially committed state.
        let handle = unsafe { &mut *runtime };
        let before = handle.runtime.clone();
        let processed = catch_unwind(AssertUnwindSafe(|| {
            handle
                .runtime
                .process_input_cascade(&block_id, input, MonotonicMs(event.monotonic_ms))
        }));
        let executions = match processed {
            Ok(Ok(executions)) => executions,
            Ok(Err(error)) => {
                handle.runtime = before;
                return Err(status_for_runtime_event(&error));
            }
            Err(_) => {
                handle.runtime = before;
                return Err(Status::Panic);
            }
        };

        let output_count = match count_effects(&executions) {
            Ok(count) => count,
            Err(status) => {
                // A logic error is a contained execution outcome. The input
                // observation remains accepted, but there is no effect list.
                // Do not roll back that semantic observation.
                return Err(status);
            }
        };
        if output_count > capacity {
            handle.runtime = before;
            // SAFETY: written was checked and points to writable storage.
            unsafe { *written = output_count };
            return Err(Status::OutputBufferTooSmall);
        }
        if output_count > 0 {
            // SAFETY: capacity was checked against output_count and the caller
            // supplied a writable buffer for all records.
            let effects = unsafe { slice::from_raw_parts_mut(effects, output_count) };
            if let Err(status) = write_effects(&executions, effects) {
                handle.runtime = before;
                return Err(status);
            }
        }
        // SAFETY: written was checked and points to writable storage.
        unsafe { *written = output_count };
        Ok(Status::Ok)
    })
}

/// Process at most one globally earliest due timer and return its effects.
/// Buffer exhaustion restores the complete runtime, including the consumed
/// timer, so the caller can retry with a larger buffer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn logiksmith_runtime_process_due_timer(
    runtime: *mut RuntimeHandle,
    monotonic_ms: u64,
    effects: *mut EffectRepr,
    capacity: usize,
    written: *mut usize,
) -> i32 {
    process_runtime_operation(runtime, effects, capacity, written, |runtime| {
        runtime
            .process_next_due_timer_cascade(MonotonicMs(monotonic_ms))
            .map(|executions| executions)
    })
}

/// Validate a source without changing the runtime. The revision is the core's
/// deterministic source-content revision, not a document revision.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn logiksmith_runtime_validate_source(
    source: *const u8,
    source_len: usize,
    revision: *mut u64,
) -> i32 {
    if revision.is_null() {
        return Status::NullPointer.code();
    }
    // SAFETY: checked output pointer; initialize all failure paths.
    unsafe { *revision = 0 };
    ffi_status(|| {
        let revision_value = with_utf8(source, source_len, MAX_SOURCE_BYTES, |source| {
            Runtime::validate_source(source).map_err(|_| Status::InvalidSource)
        })?;
        // SAFETY: revision is non-null and caller-owned writable storage.
        unsafe { *revision = revision_value };
        Ok(Status::Ok)
    })
}

/// Simulate an input against a copy of the block's current inputs, state and
/// pending timers. The optional source is installed only on a throwaway core
/// clone, and effects are never sent through the live output path.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn logiksmith_runtime_simulate_input(
    runtime: *mut RuntimeHandle,
    event: *const InputEventRepr,
    source: *const u8,
    source_len: usize,
    effects: *mut EffectRepr,
    capacity: usize,
    written: *mut usize,
) -> i32 {
    if written.is_null() {
        return Status::NullPointer.code();
    }
    unsafe { *written = 0 };
    if runtime.is_null() || event.is_null() {
        return Status::NullPointer.code();
    }
    if capacity > MAX_EFFECTS || (capacity > 0 && effects.is_null()) {
        return if capacity > MAX_EFFECTS {
            Status::InvalidLength.code()
        } else {
            Status::NullPointer.code()
        };
    }
    ffi_status(|| {
        // SAFETY: pointers checked above and readable for this call.
        let event = unsafe { &*event };
        let block_id = decode_block_id(event.block_id, event.block_id_len)?;
        let endpoint = decode_endpoint_name(event.endpoint, event.endpoint_len)?;
        let value = decode_value(event.value)?;
        let handle = unsafe { &*runtime };
        let snapshot = handle
            .runtime
            .block(&block_id)
            .ok_or(Status::UnknownBlock)?
            .snapshot_at(MonotonicMs(event.monotonic_ms));
        let mut inputs: Vec<SimulationInput> = snapshot
            .inputs
            .iter()
            .map(simulation_input_from_snapshot)
            .collect();
        let Some(trigger_input) = inputs.iter_mut().find(|input| input.endpoint == endpoint) else {
            return Err(Status::UnknownEndpoint);
        };
        trigger_input.value = Some(value);
        trigger_input.valid = true;
        trigger_input.age_ms = Some(0);
        let scenario = SimulationScenario {
            trigger: SimulationTrigger {
                endpoint: endpoint.clone(),
                value,
                previous: snapshot
                    .inputs
                    .iter()
                    .find(|input| input.endpoint == endpoint)
                    .and_then(|input| input.value),
            },
            inputs,
        };
        let result = if source_len == 0 && source.is_null() {
            handle.runtime.simulate_input(&block_id, scenario)
        } else {
            let source = with_utf8(source, source_len, MAX_SOURCE_BYTES, |source| {
                Ok(source.to_owned())
            })?;
            handle.runtime.simulate_input_with_source(
                &block_id,
                source,
                scenario,
                snapshot.state,
                snapshot.pending_timers,
                MonotonicMs(event.monotonic_ms),
            )
        };
        let execution = result.map_err(|_| Status::InvalidConfiguration)?;
        write_execution_effects(&[execution], effects, capacity, written)
    })
}

/// Simulate one named timer against copied block state. `timer` is a bounded
/// UTF-8 name; no live timer is consumed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn logiksmith_runtime_simulate_timer(
    runtime: *mut RuntimeHandle,
    block_id: *const u8,
    block_id_len: usize,
    timer: *const u8,
    timer_len: usize,
    fired_at_ms: u64,
    source: *const u8,
    source_len: usize,
    effects: *mut EffectRepr,
    capacity: usize,
    written: *mut usize,
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
        let block_id = decode_block_id(block_id, block_id_len)?;
        let timer = with_utf8(timer, timer_len, MAX_IDENTIFIER_BYTES, |timer| {
            timer.parse().map_err(|_| Status::UnknownTimer)
        })?;
        let handle = unsafe { &*runtime };
        let snapshot = handle
            .runtime
            .block(&block_id)
            .ok_or(Status::UnknownBlock)?
            .snapshot_at(MonotonicMs(fired_at_ms));
        if !snapshot
            .pending_timers
            .iter()
            .any(|pending| pending.name == timer)
        {
            return Err(Status::UnknownTimer);
        }
        let scenario = TimerSimulationScenario {
            timer,
            fired_at: MonotonicMs(fired_at_ms),
            inputs: snapshot
                .inputs
                .iter()
                .map(simulation_input_from_snapshot)
                .collect(),
            state: snapshot.state.clone(),
            pending_timers: snapshot.pending_timers.clone(),
        };
        let execution = if source_len == 0 && source.is_null() {
            handle.runtime.simulate_timer(&block_id, scenario)
        } else {
            let source = with_utf8(source, source_len, MAX_SOURCE_BYTES, |source| {
                Ok(source.to_owned())
            })?;
            handle
                .runtime
                .simulate_timer_with_source(&block_id, source, scenario)
        }
        .map_err(|_| Status::InvalidConfiguration)?;
        write_execution_effects(&[execution], effects, capacity, written)
    })
}

/// Activate source and/or enabled state after CAS checks. Core validation is
/// completed before mutation; document revision is advanced only on success.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn logiksmith_runtime_activate(
    runtime: *mut RuntimeHandle,
    activation: *const ActivationRepr,
    result: *mut ActivationResultRepr,
) -> i32 {
    if result.is_null() {
        return Status::NullPointer.code();
    }
    unsafe { *result = ActivationResultRepr::default() };
    if runtime.is_null() || activation.is_null() {
        return Status::NullPointer.code();
    }
    ffi_status(|| {
        // SAFETY: pointers checked above and caller owns their readable data.
        let activation = unsafe { &*activation };
        if activation.reserved != 0
            || activation.has_source > 1
            || activation.has_enabled > 1
            || (activation.has_enabled == 1 && activation.enabled > 1)
            || (activation.has_source == 0 && activation.has_enabled == 0)
        {
            return Err(Status::InvalidConfiguration);
        }
        let handle = unsafe { &mut *runtime };
        check_revision(
            handle.document_revision,
            activation.expected_document_revision,
        )?;
        check_revision(
            handle.structural_revision,
            activation.expected_structural_revision,
        )?;
        let block_id = decode_block_id(activation.block_id, activation.block_id_len)?;
        let expected_block = handle
            .runtime
            .block(&block_id)
            .ok_or(Status::UnknownBlock)?
            .active_logic_revision();
        check_revision(expected_block, activation.expected_block_revision)?;
        let source = if activation.has_source == 1 {
            Some(with_utf8(
                activation.source,
                activation.source_len,
                MAX_SOURCE_BYTES,
                |source| Ok(source.to_owned()),
            )?)
        } else {
            None
        };
        let candidate = RuntimeActivation::single(logiksmith_core::BlockActivation::new(
            block_id.clone(),
            source,
            (activation.has_enabled == 1).then_some(activation.enabled != 0),
        ));
        let activated = handle
            .runtime
            .activate(candidate)
            .map_err(|_| Status::InvalidSource)?;
        let block = activated
            .blocks
            .first()
            .ok_or(Status::InvalidConfiguration)?;
        if block.source_changed || block.enabled_changed {
            handle.document_revision = handle
                .document_revision
                .checked_add(1)
                .ok_or(Status::RuntimeLimit)?;
        }
        // SAFETY: result is non-null and caller-owned writable storage.
        unsafe {
            (*result).block_id_len =
                copy_identifier_len(&mut (*result).block_id, block.block_id.as_str())?;
            (*result).logic_revision = block.logic_revision;
            (*result).document_revision = handle.document_revision;
            (*result).structural_revision = handle.structural_revision;
            (*result).enabled = u8::from(block.enabled);
            (*result).source_changed = u8::from(block.source_changed);
            (*result).enabled_changed = u8::from(block.enabled_changed);
            (*result).cancelled_timer_count = block.cancelled_timers.len();
        }
        Ok(Status::Ok)
    })
}

/// Enable/disable one block with the same revision and cancellation semantics
/// as [`logiksmith_runtime_activate`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn logiksmith_runtime_set_enabled(
    runtime: *mut RuntimeHandle,
    block_id: *const u8,
    block_id_len: usize,
    enabled: u8,
    expected_document_revision: u64,
    expected_structural_revision: u64,
    expected_block_revision: u64,
    result: *mut ActivationResultRepr,
) -> i32 {
    if enabled > 1 {
        return Status::InvalidValue.code();
    }
    let block = ActivationRepr {
        block_id,
        block_id_len,
        source: std::ptr::null(),
        source_len: 0,
        has_source: 0,
        enabled,
        has_enabled: 1,
        reserved: 0,
        expected_document_revision,
        expected_structural_revision,
        expected_block_revision,
    };
    // SAFETY: this forwards the caller's validated pointers and result slot.
    unsafe { logiksmith_runtime_activate(runtime, &block, result) }
}

/// Clear operational suspension for one block. Resuming never restores timers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn logiksmith_runtime_resume(
    runtime: *mut RuntimeHandle,
    block_id: *const u8,
    block_id_len: usize,
    expected_document_revision: u64,
    expected_structural_revision: u64,
    expected_block_revision: u64,
) -> i32 {
    if runtime.is_null() {
        return Status::NullPointer.code();
    }
    ffi_status(|| {
        let handle = unsafe { &mut *runtime };
        check_revision(handle.document_revision, expected_document_revision)?;
        check_revision(handle.structural_revision, expected_structural_revision)?;
        let block_id = decode_block_id(block_id, block_id_len)?;
        let current = handle
            .runtime
            .block(&block_id)
            .ok_or(Status::UnknownBlock)?
            .active_logic_revision();
        check_revision(current, expected_block_revision)?;
        handle
            .runtime
            .resume_block(&block_id)
            .map_err(|_| Status::UnknownBlock)?;
        Ok(Status::Ok)
    })
}

/// Return the bounded runtime usage/revision projection without allocating in C.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn logiksmith_runtime_project(
    runtime: *const RuntimeHandle,
    result: *mut RuntimeProjectionRepr,
) -> i32 {
    if runtime.is_null() || result.is_null() {
        return Status::NullPointer.code();
    }
    ffi_status(|| {
        let handle = unsafe { &*runtime };
        let usage = handle.runtime.usage();
        let last = handle.runtime.last_accepted_at();
        // SAFETY: result is non-null and caller-owned writable storage.
        unsafe {
            *result = RuntimeProjectionRepr {
                document_revision: handle.document_revision,
                structural_revision: handle.structural_revision,
                last_accepted_ms: last.map_or(0, |value| value.0),
                has_last_accepted: u8::from(last.is_some()),
                reserved: [0; 7],
                block_count: usage.logic_blocks,
                signal_count: usage.signals,
                signal_bindings: usage.signal_bindings,
                logic_source_bytes: usage.logic_source_bytes,
                state_entries: usage.state_entries,
                state_bytes: usage.state_bytes,
                pending_timers: usage.pending_timers,
            };
        }
        Ok(Status::Ok)
    })
}

/// Return one compact block projection. If `result` is null the required
/// serialized projection size is returned through `written` without mutation.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn logiksmith_runtime_project_block(
    runtime: *const RuntimeHandle,
    block_id: *const u8,
    block_id_len: usize,
    result: *mut BlockProjectionRepr,
    written: *mut usize,
) -> i32 {
    if written.is_null() || runtime.is_null() {
        return Status::NullPointer.code();
    }
    unsafe { *written = 0 };
    ffi_status(|| {
        let block_id = decode_block_id(block_id, block_id_len)?;
        let handle = unsafe { &*runtime };
        let snapshot = handle
            .runtime
            .block(&block_id)
            .ok_or(Status::UnknownBlock)?
            .snapshot_at(handle.runtime.last_accepted_at().unwrap_or_default());
        let required = snapshot
            .inputs
            .len()
            .saturating_add(snapshot.pending_timers.len());
        if snapshot.inputs.len() > MAX_INPUTS_PER_BLOCK
            || snapshot.pending_timers.len() > MAX_TIMERS_PER_BLOCK
        {
            unsafe { *written = required };
            return Err(Status::RuntimeLimit);
        }
        if result.is_null() {
            unsafe { *written = 1 };
            return Err(Status::BufferTooSmall);
        }
        let destination = unsafe { &mut *result };
        *destination = BlockProjectionRepr::default();
        destination.block_id_len =
            copy_identifier_len(&mut destination.block_id, snapshot.id.as_str())?;
        destination.health = health_code(snapshot.health);
        destination.enabled = u8::from(snapshot.enabled);
        destination.logic_revision = snapshot.logic_revision;
        destination.consecutive_failures = snapshot.consecutive_failures;
        destination.input_count = snapshot.inputs.len();
        destination.timer_count = snapshot.pending_timers.len();
        for (index, input) in snapshot.inputs.iter().enumerate() {
            destination.inputs[index] = input_repr(input)?;
        }
        for (index, timer) in snapshot.pending_timers.iter().enumerate() {
            destination.timers[index] = timer_repr(timer)?;
        }
        unsafe { *written = 1 };
        Ok(Status::Ok)
    })
}
