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

    #[test]
    fn projection_validation_simulation_and_revision_gate_are_bounded() {
        let (runtime, fixture) = create_fixture();
        let mut projection = RuntimeProjectionRepr::default();
        assert_eq!(
            unsafe { logiksmith_runtime_project(runtime, &mut projection) },
            Status::Ok.code()
        );
        assert_eq!(projection.document_revision, 1);
        assert_eq!(projection.structural_revision, 1);
        assert_eq!(projection.block_count, 1);
        assert_eq!(projection.has_last_accepted, 0);

        let mut block = BlockProjectionRepr::default();
        let mut written = 0;
        assert_eq!(
            unsafe {
                logiksmith_runtime_project_block(
                    runtime,
                    b"main".as_ptr(),
                    4,
                    &mut block,
                    &mut written,
                )
            },
            Status::Ok.code()
        );
        assert_eq!(written, 1);
        assert_eq!(block.input_count, 1);
        assert_eq!(&block.inputs[0].endpoint[..6], b"switch");

        let mut effects = [EffectRepr::default(); 1];
        assert_eq!(
            unsafe {
                logiksmith_runtime_simulate_input(
                    runtime,
                    &fixture.event,
                    ptr::null(),
                    0,
                    effects.as_mut_ptr(),
                    effects.len(),
                    &mut written,
                )
            },
            Status::Ok.code()
        );
        assert_eq!(written, 1);
        assert_eq!(projection_after(runtime).has_last_accepted, 0);

        let source = b"function handle(event) return {} end";
        let activation = ActivationRepr {
            block_id: b"main".as_ptr(),
            block_id_len: 4,
            source: source.as_ptr(),
            source_len: source.len(),
            has_source: 1,
            enabled: 0,
            has_enabled: 0,
            reserved: 0,
            expected_document_revision: 1,
            expected_structural_revision: 1,
            expected_block_revision: block.logic_revision,
        };
        let mut result = ActivationResultRepr::default();
        assert_eq!(
            unsafe { logiksmith_runtime_activate(runtime, &activation, &mut result) },
            Status::Ok.code()
        );
        assert_eq!(result.document_revision, 2);
        assert_eq!(result.source_changed, 1);
        assert_eq!(
            unsafe { logiksmith_runtime_activate(runtime, &activation, &mut result) },
            Status::RevisionConflict.code()
        );
        assert_eq!(projection_after(runtime).document_revision, 2);
        assert_eq!(
            unsafe { logiksmith_runtime_destroy(runtime) },
            Status::Ok.code()
        );
    }

    #[test]
    fn toml_creation_uses_shared_embedded_document_validation() {
        let source = br#"[[blocks]]
id = "main"
revision = 4
enabled = true
source = "function handle(event) return { outputs = { light = event.value } } end"

[[blocks.inputs]]
name = "trigger"
dpt = "1.001"

[[blocks.outputs]]
name = "light"
dpt = "1.001"

[[blocks.knx_bindings]]
endpoint = "trigger"
group_address = "1/2/3"

[[blocks.knx_bindings]]
endpoint = "light"
group_address = "1/2/4"
"#;
        let document = RuntimeDocumentBytesRepr {
            bytes: source.as_ptr(),
            len: source.len(),
            document_revision: 7,
            structural_revision: 8,
        };
        let mut runtime = ptr::null_mut();
        assert_eq!(
            unsafe { logiksmith_runtime_create_from_toml(&document, &mut runtime) },
            Status::Ok.code()
        );
        let projection = projection_after(runtime);
        assert_eq!(projection.document_revision, 7);
        assert_eq!(projection.structural_revision, 8);
        assert_eq!(projection.block_count, 1);
        assert_eq!(
            unsafe { logiksmith_runtime_destroy(runtime) },
            Status::Ok.code()
        );
    }

    fn projection_after(runtime: *const RuntimeHandle) -> RuntimeProjectionRepr {
        let mut projection = RuntimeProjectionRepr::default();
        assert_eq!(
            unsafe { logiksmith_runtime_project(runtime, &mut projection) },
            Status::Ok.code()
        );
        projection
    }
}
