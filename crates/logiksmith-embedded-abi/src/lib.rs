//! Small, versioned C ABI for hosting the portable LogikSmith runtime.
//!
//! The ABI deliberately stops at logical endpoint effects. An OpenKNX host
//! owns group-address bindings and translates each effect to a KNX telegram;
//! no KNX or OpenKNX type is used here. Strings crossing the boundary are
//! pointer/length byte slices and outputs use caller-owned fixed records.

use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    slice, str,
};

use logiksmith_core::{
    BlockConfig, BlockId, Dpt, Endpoint, EndpointDirection, EndpointName, InputEvent, MonotonicMs,
    Runtime, RuntimeConfig, RuntimeEventError, TypedValue, Value,
};

pub const ABI_VERSION: u32 = 1;
pub const MAX_IDENTIFIER_BYTES: usize = 64;
pub const MAX_SOURCE_BYTES: usize = logiksmith_core::MAX_LOGIC_SOURCE_BYTES;
pub const MAX_BLOCKS: usize = 64;
pub const MAX_ENDPOINTS_PER_BLOCK: usize = 64;
pub const MAX_EFFECTS: usize = 256;

const INPUT: u8 = 0;
const OUTPUT: u8 = 1;
const VALUE_BOOL: u8 = 1;
const VALUE_PERCENT: u8 = 2;
const VALUE_TEMPERATURE_CENTI_DEGREES: u8 = 3;
const MAX_KNX_GROUP_ADDRESS: u16 = 0x7fff;

/// ABI result values. Numeric discriminants are part of the public contract.
#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Status {
    Ok = 0,
    NullPointer = 1,
    InvalidLength = 2,
    InvalidUtf8 = 3,
    InvalidBlockId = 4,
    InvalidEndpointName = 5,
    InvalidDpt = 6,
    InvalidValue = 7,
    InvalidConfiguration = 8,
    UnknownBlock = 9,
    UnknownEndpoint = 10,
    EndpointNotInput = 11,
    DptMismatch = 12,
    TimeWentBackwards = 13,
    LogicError = 14,
    OutputBufferTooSmall = 15,
    Panic = 16,
    InvalidGroupAddress = 17,
    RuntimeLimit = 18,
}

impl Status {
    const fn code(self) -> i32 {
        self as i32
    }
}

/// ABI value representation. `scalar` is interpreted according to `kind`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ValueRepr {
    pub dpt_major: u16,
    pub dpt_subtype: u16,
    pub kind: u8,
    pub reserved: u8,
    pub reserved2: u16,
    pub scalar: i32,
}

/// Endpoint declaration used when constructing an opaque runtime.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct EndpointConfig {
    pub name: *const u8,
    pub name_len: usize,
    pub direction: u8,
    pub reserved: [u8; 3],
    pub dpt_major: u16,
    pub dpt_subtype: u16,
}

/// One block declaration used when constructing an opaque runtime.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct BlockConfigRepr {
    pub block_id: *const u8,
    pub block_id_len: usize,
    pub logic_source: *const u8,
    pub logic_source_len: usize,
    pub endpoints: *const EndpointConfig,
    pub endpoint_count: usize,
}

/// Complete configuration for [`runtime_create`].
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RuntimeConfigRepr {
    pub blocks: *const BlockConfigRepr,
    pub block_count: usize,
}

/// A triggering logical input event. Address fields are metadata for the host
/// and are not interpreted by the transport-neutral core.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct InputEventRepr {
    pub block_id: *const u8,
    pub block_id_len: usize,
    pub endpoint: *const u8,
    pub endpoint_len: usize,
    pub value: ValueRepr,
    pub source_address: u16,
    pub group_address: u16,
    pub monotonic_ms: u64,
}

/// Caller-owned effect record. Names are copied into these fixed arrays.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EffectRepr {
    pub block_id: [u8; MAX_IDENTIFIER_BYTES],
    pub block_id_len: u16,
    pub endpoint: [u8; MAX_IDENTIFIER_BYTES],
    pub endpoint_len: u16,
    pub value: ValueRepr,
}

impl Default for EffectRepr {
    fn default() -> Self {
        Self {
            block_id: [0; MAX_IDENTIFIER_BYTES],
            block_id_len: 0,
            endpoint: [0; MAX_IDENTIFIER_BYTES],
            endpoint_len: 0,
            value: ValueRepr::default(),
        }
    }
}

/// Opaque runtime allocated by [`runtime_create`].
#[repr(C)]
pub struct RuntimeHandle {
    runtime: Runtime,
}

/// Return the ABI version expected by the C header.
#[unsafe(no_mangle)]
pub extern "C" fn logiksmith_abi_version() -> u32 {
    ABI_VERSION
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
        let runtime = Runtime::try_new(RuntimeConfig::new(block_configs))
            .map_err(|_| Status::InvalidConfiguration)?;
        let handle = Box::new(RuntimeHandle { runtime });
        // SAFETY: out_runtime is non-null and caller supplied writable storage.
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::{panic::catch_unwind, ptr};

    fn endpoint(name: &'static [u8], direction: u8) -> EndpointConfig {
        EndpointConfig {
            name: name.as_ptr(),
            name_len: name.len(),
            direction,
            reserved: [0; 3],
            dpt_major: 1,
            dpt_subtype: 1,
        }
    }

    fn fixture() -> (RuntimeConfigRepr, LogiksmithInputEventFixture) {
        let endpoints = Box::leak(Box::new([
            endpoint(b"switch", INPUT),
            endpoint(b"light", OUTPUT),
        ]));
        let source = Box::leak(
            b"function handle(event, input)\n  if event.input == 'switch' then\n    return { outputs = { light = event.value } }\n  end\nend\0".to_vec().into_boxed_slice(),
        );
        let block = Box::leak(Box::new(BlockConfigRepr {
            block_id: b"main".as_ptr(),
            block_id_len: 4,
            logic_source: source.as_ptr(),
            logic_source_len: source.len() - 1,
            endpoints: endpoints.as_ptr(),
            endpoint_count: endpoints.len(),
        }));
        let config = RuntimeConfigRepr {
            blocks: block,
            block_count: 1,
        };
        let event = LogiksmithInputEventFixture {
            event: InputEventRepr {
                block_id: b"main".as_ptr(),
                block_id_len: 4,
                endpoint: b"switch".as_ptr(),
                endpoint_len: 6,
                value: ValueRepr {
                    dpt_major: 1,
                    dpt_subtype: 1,
                    kind: VALUE_BOOL,
                    reserved: 0,
                    reserved2: 0,
                    scalar: 1,
                },
                source_address: 0x1101,
                group_address: 0x1234,
                monotonic_ms: 1,
            },
            _endpoints: endpoints,
            _source: source,
            _block: block,
        };
        (config, event)
    }

    struct LogiksmithInputEventFixture {
        event: InputEventRepr,
        _endpoints: &'static mut [EndpointConfig; 2],
        _source: &'static mut [u8],
        _block: &'static mut BlockConfigRepr,
    }

    fn create_fixture() -> (*mut RuntimeHandle, LogiksmithInputEventFixture) {
        let (config, event) = fixture();
        let mut runtime = ptr::null_mut();
        let status = unsafe { logiksmith_runtime_create(&config, &mut runtime) };
        assert_eq!(status, Status::Ok.code());
        (runtime, event)
    }

    #[test]
    fn abi_version_and_status_numbers_are_stable() {
        assert_eq!(logiksmith_abi_version(), ABI_VERSION);
        assert_eq!(Status::Ok.code(), 0);
        assert_eq!(Status::OutputBufferTooSmall.code(), 15);
        assert_eq!(Status::Panic.code(), 16);
        assert_eq!(Status::RuntimeLimit.code(), 18);
    }

    #[test]
    fn m13_runtime_limits_have_a_stable_abi_status() {
        assert_eq!(
            status_for_runtime_event(&RuntimeEventError::CascadeLimit {
                actual: 2,
                maximum: 1,
            }),
            Status::RuntimeLimit
        );
        assert_eq!(
            status_for_runtime_event(&RuntimeEventError::ResourceLimit {
                resource: "state",
                actual: 2,
                maximum: 1,
            }),
            Status::RuntimeLimit
        );
        assert_eq!(
            status_for_runtime_event(&RuntimeEventError::CascadeTimeLimit {
                elapsed_ms: 2,
                maximum_ms: 1,
            }),
            Status::RuntimeLimit
        );
    }

    #[test]
    fn valid_event_is_processed_into_caller_owned_typed_effect() {
        let (runtime, fixture) = create_fixture();
        let mut effects = [EffectRepr::default(); 1];
        let mut written = 0;
        let status = unsafe {
            logiksmith_runtime_process_input(
                runtime,
                &fixture.event,
                effects.as_mut_ptr(),
                effects.len(),
                &mut written,
            )
        };
        assert_eq!(status, Status::Ok.code());
        assert_eq!(written, 1);
        assert_eq!(
            &effects[0].block_id[..effects[0].block_id_len as usize],
            b"main"
        );
        assert_eq!(
            &effects[0].endpoint[..effects[0].endpoint_len as usize],
            b"light"
        );
        assert_eq!(effects[0].value, fixture.event.value);
        assert_eq!(
            unsafe { logiksmith_runtime_destroy(runtime) },
            Status::Ok.code()
        );
    }

    #[test]
    fn malformed_pointers_and_values_are_rejected_without_unwinding() {
        let (runtime, fixture) = create_fixture();
        let mut written = 99;
        let mut event = fixture.event;
        event.endpoint = ptr::null();
        event.endpoint_len = 1;
        let result = catch_unwind(AssertUnwindSafe(|| unsafe {
            logiksmith_runtime_process_input(runtime, &event, ptr::null_mut(), 0, &mut written)
        }));
        assert_eq!(result.unwrap(), Status::NullPointer.code());
        assert_eq!(written, 0);

        event = fixture.event;
        event.endpoint_len = MAX_IDENTIFIER_BYTES + 1;
        assert_eq!(
            unsafe {
                logiksmith_runtime_process_input(runtime, &event, ptr::null_mut(), 0, &mut written)
            },
            Status::InvalidLength.code()
        );

        event = fixture.event;
        event.endpoint = b"Switch".as_ptr();
        event.endpoint_len = 6;
        assert_eq!(
            unsafe {
                logiksmith_runtime_process_input(runtime, &event, ptr::null_mut(), 0, &mut written)
            },
            Status::InvalidEndpointName.code()
        );

        event = fixture.event;
        event.endpoint = [0xff].as_ptr();
        event.endpoint_len = 1;
        assert_eq!(
            unsafe {
                logiksmith_runtime_process_input(runtime, &event, ptr::null_mut(), 0, &mut written)
            },
            Status::InvalidUtf8.code()
        );

        event = fixture.event;
        event.value.scalar = 2;
        assert_eq!(
            unsafe {
                logiksmith_runtime_process_input(runtime, &event, ptr::null_mut(), 0, &mut written)
            },
            Status::InvalidValue.code()
        );

        event = fixture.event;
        event.group_address = MAX_KNX_GROUP_ADDRESS + 1;
        assert_eq!(
            unsafe {
                logiksmith_runtime_process_input(runtime, &event, ptr::null_mut(), 0, &mut written)
            },
            Status::InvalidGroupAddress.code()
        );

        event = fixture.event;
        event.value = ValueRepr {
            dpt_major: 5,
            dpt_subtype: 1,
            kind: VALUE_PERCENT,
            reserved: 0,
            reserved2: 0,
            scalar: 50,
        };
        assert_eq!(
            unsafe {
                logiksmith_runtime_process_input(runtime, &event, ptr::null_mut(), 0, &mut written)
            },
            Status::DptMismatch.code()
        );

        let mut oversized_effects = [EffectRepr::default(); MAX_EFFECTS + 1];
        assert_eq!(
            unsafe {
                logiksmith_runtime_process_input(
                    runtime,
                    &fixture.event,
                    oversized_effects.as_mut_ptr(),
                    MAX_EFFECTS + 1,
                    &mut written,
                )
            },
            Status::InvalidLength.code()
        );
        assert_eq!(
            unsafe { logiksmith_runtime_destroy(runtime) },
            Status::Ok.code()
        );
    }

    #[test]
    fn undersized_effect_buffer_rolls_back_and_reports_required_capacity() {
        let (runtime, fixture) = create_fixture();
        let mut written = 0;
        let status = unsafe {
            logiksmith_runtime_process_input(
                runtime,
                &fixture.event,
                ptr::null_mut(),
                0,
                &mut written,
            )
        };
        assert_eq!(status, Status::OutputBufferTooSmall.code());
        assert_eq!(written, 1);

        let mut effects = [EffectRepr::default(); 1];
        let status = unsafe {
            logiksmith_runtime_process_input(
                runtime,
                &fixture.event,
                effects.as_mut_ptr(),
                effects.len(),
                &mut written,
            )
        };
        assert_eq!(status, Status::Ok.code());
        assert_eq!(written, 1);
        assert_eq!(
            unsafe { logiksmith_runtime_destroy(runtime) },
            Status::Ok.code()
        );
    }

    #[test]
    fn malformed_configuration_is_rejected_before_handle_creation() {
        let (mut oversized_config, _fixture) = fixture();
        oversized_config.block_count = MAX_BLOCKS + 1;
        let mut runtime = ptr::null_mut();
        assert_eq!(
            unsafe { logiksmith_runtime_create(&oversized_config, &mut runtime) },
            Status::InvalidLength.code()
        );
        assert!(runtime.is_null());

        let config = RuntimeConfigRepr {
            blocks: ptr::null(),
            block_count: 1,
        };
        let mut runtime = ptr::null_mut();
        assert_eq!(
            unsafe { logiksmith_runtime_create(&config, &mut runtime) },
            Status::NullPointer.code()
        );
        assert!(runtime.is_null());
    }
}
