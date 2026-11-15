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
    InvalidSource = 19,
    RevisionConflict = 20,
    UnknownTimer = 21,
    BufferTooSmall = 22,
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
    document_revision: u64,
    structural_revision: u64,
}

/// A document wrapper used by hosts which already validated the configuration
/// through the shared document layer. The ABI copies all data before returning.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RuntimeDocumentRepr {
    pub config: *const RuntimeConfigRepr,
    pub document_revision: u64,
    pub structural_revision: u64,
}

/// Caller-owned TOML document envelope. Parsing and embedded capability
/// validation are delegated to the shared runtime API before any handle is
/// created.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RuntimeDocumentBytesRepr {
    pub bytes: *const u8,
    pub len: usize,
    pub document_revision: u64,
    pub structural_revision: u64,
}

/// One bounded source/enabled activation update.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ActivationRepr {
    pub block_id: *const u8,
    pub block_id_len: usize,
    pub source: *const u8,
    pub source_len: usize,
    pub has_source: u8,
    pub enabled: u8,
    pub has_enabled: u8,
    pub reserved: u8,
    pub expected_document_revision: u64,
    pub expected_structural_revision: u64,
    pub expected_block_revision: u64,
}

/// Result of one activation. Timer names are copied into caller-owned slots.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ActivationResultRepr {
    pub block_id: [u8; MAX_IDENTIFIER_BYTES],
    pub block_id_len: u16,
    pub reserved: u16,
    pub logic_revision: u64,
    pub document_revision: u64,
    pub structural_revision: u64,
    pub enabled: u8,
    pub source_changed: u8,
    pub enabled_changed: u8,
    pub reserved2: u8,
    pub cancelled_timer_count: usize,
}

impl Default for ActivationResultRepr {
    fn default() -> Self {
        Self {
            block_id: [0; MAX_IDENTIFIER_BYTES],
            block_id_len: 0,
            reserved: 0,
            logic_revision: 0,
            document_revision: 0,
            structural_revision: 0,
            enabled: 0,
            source_changed: 0,
            enabled_changed: 0,
            reserved2: 0,
            cancelled_timer_count: 0,
        }
    }
}

/// One pending timer projection. It has no borrowed runtime memory.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct TimerRepr {
    pub name: [u8; MAX_IDENTIFIER_BYTES],
    pub name_len: u16,
    pub reserved: u16,
    pub scheduled_at: u64,
    pub due_at: u64,
    pub scheduled_logic_revision: u64,
}

impl Default for TimerRepr {
    fn default() -> Self {
        Self {
            name: [0; MAX_IDENTIFIER_BYTES],
            name_len: 0,
            reserved: 0,
            scheduled_at: 0,
            due_at: 0,
            scheduled_logic_revision: 0,
        }
    }
}

/// One input projection. `valid` is zero when the input is unknown.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct InputRepr {
    pub endpoint: [u8; MAX_IDENTIFIER_BYTES],
    pub endpoint_len: u16,
    pub valid: u8,
    pub reserved: u8,
    pub value: ValueRepr,
    pub age_ms: u64,
}

impl Default for InputRepr {
    fn default() -> Self {
        Self {
            endpoint: [0; MAX_IDENTIFIER_BYTES],
            endpoint_len: 0,
            valid: 0,
            reserved: 0,
            value: ValueRepr::default(),
            age_ms: 0,
        }
    }
}

/// Compact block projection with bounded input and timer arrays.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct BlockProjectionRepr {
    pub block_id: [u8; MAX_IDENTIFIER_BYTES],
    pub block_id_len: u16,
    pub health: u8,
    pub enabled: u8,
    pub logic_revision: u64,
    pub consecutive_failures: u32,
    pub input_count: usize,
    pub timer_count: usize,
    pub inputs: [InputRepr; MAX_INPUTS_PER_BLOCK],
    pub timers: [TimerRepr; MAX_TIMERS_PER_BLOCK],
}

impl Default for BlockProjectionRepr {
    fn default() -> Self {
        Self {
            block_id: [0; MAX_IDENTIFIER_BYTES],
            block_id_len: 0,
            health: 0,
            enabled: 0,
            logic_revision: 0,
            consecutive_failures: 0,
            input_count: 0,
            timer_count: 0,
            inputs: [InputRepr::default(); MAX_INPUTS_PER_BLOCK],
            timers: [TimerRepr::default(); MAX_TIMERS_PER_BLOCK],
        }
    }
}

/// Runtime usage and revision projection.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RuntimeProjectionRepr {
    pub document_revision: u64,
    pub structural_revision: u64,
    pub last_accepted_ms: u64,
    pub has_last_accepted: u8,
    pub reserved: [u8; 7],
    pub block_count: usize,
    pub signal_count: usize,
    pub signal_bindings: usize,
    pub logic_source_bytes: usize,
    pub state_entries: usize,
    pub state_bytes: usize,
    pub pending_timers: usize,
}
