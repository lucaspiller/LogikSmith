#ifndef LOGIKSMITH_EMBEDDED_ABI_H
#define LOGIKSMITH_EMBEDDED_ABI_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

#define LOGIKSMITH_ABI_VERSION 2u
#define LOGIKSMITH_MAX_IDENTIFIER_BYTES 64u
#define LOGIKSMITH_MAX_SOURCE_BYTES (64u * 1024u)
#define LOGIKSMITH_MAX_BLOCKS 64u
#define LOGIKSMITH_MAX_ENDPOINTS_PER_BLOCK 64u
#define LOGIKSMITH_MAX_EFFECTS 256u
#define LOGIKSMITH_MAX_INPUTS_PER_BLOCK LOGIKSMITH_MAX_ENDPOINTS_PER_BLOCK
#define LOGIKSMITH_MAX_TIMERS_PER_BLOCK 32u
#define LOGIKSMITH_MAX_DOCUMENT_BYTES (16u * 1024u)

/* Stable numeric results returned by every fallible ABI operation. */
enum LogiksmithStatus {
    LOGIKSMITH_STATUS_OK = 0,
    LOGIKSMITH_STATUS_NULL_POINTER = 1,
    LOGIKSMITH_STATUS_INVALID_LENGTH = 2,
    LOGIKSMITH_STATUS_INVALID_UTF8 = 3,
    LOGIKSMITH_STATUS_INVALID_BLOCK_ID = 4,
    LOGIKSMITH_STATUS_INVALID_ENDPOINT_NAME = 5,
    LOGIKSMITH_STATUS_INVALID_DPT = 6,
    LOGIKSMITH_STATUS_INVALID_VALUE = 7,
    LOGIKSMITH_STATUS_INVALID_CONFIGURATION = 8,
    LOGIKSMITH_STATUS_UNKNOWN_BLOCK = 9,
    LOGIKSMITH_STATUS_UNKNOWN_ENDPOINT = 10,
    LOGIKSMITH_STATUS_ENDPOINT_NOT_INPUT = 11,
    LOGIKSMITH_STATUS_DPT_MISMATCH = 12,
    LOGIKSMITH_STATUS_TIME_WENT_BACKWARDS = 13,
    LOGIKSMITH_STATUS_LOGIC_ERROR = 14,
    LOGIKSMITH_STATUS_OUTPUT_BUFFER_TOO_SMALL = 15,
    LOGIKSMITH_STATUS_PANIC = 16,
    LOGIKSMITH_STATUS_INVALID_GROUP_ADDRESS = 17,
    LOGIKSMITH_STATUS_RUNTIME_LIMIT = 18,
    LOGIKSMITH_STATUS_INVALID_SOURCE = 19,
    LOGIKSMITH_STATUS_REVISION_CONFLICT = 20,
    LOGIKSMITH_STATUS_UNKNOWN_TIMER = 21,
    LOGIKSMITH_STATUS_BUFFER_TOO_SMALL = 22,
};

enum LogiksmithEndpointDirection {
    LOGIKSMITH_ENDPOINT_INPUT = 0,
    LOGIKSMITH_ENDPOINT_OUTPUT = 1,
};

enum LogiksmithValueKind {
    LOGIKSMITH_VALUE_BOOL = 1,
    LOGIKSMITH_VALUE_PERCENT = 2,
    LOGIKSMITH_VALUE_TEMPERATURE_CENTI_DEGREES = 3,
};

typedef struct LogiksmithValue {
    uint16_t dpt_major;
    uint16_t dpt_subtype;
    uint8_t kind;
    uint8_t reserved;
    uint16_t reserved2;
    /* bool is 0 or 1, percent is 0..100, temperature is signed centi-degrees. */
    int32_t scalar;
} LogiksmithValue;

typedef struct LogiksmithEndpointConfig {
    const uint8_t *name;
    size_t name_len;
    uint8_t direction;
    uint8_t reserved[3];
    uint16_t dpt_major;
    uint16_t dpt_subtype;
} LogiksmithEndpointConfig;

typedef struct LogiksmithBlockConfig {
    const uint8_t *block_id;
    size_t block_id_len;
    const uint8_t *logic_source;
    size_t logic_source_len;
    const LogiksmithEndpointConfig *endpoints;
    size_t endpoint_count;
} LogiksmithBlockConfig;

typedef struct LogiksmithRuntimeConfig {
    const LogiksmithBlockConfig *blocks;
    size_t block_count;
} LogiksmithRuntimeConfig;

/* A validated document envelope. The nested configuration is copied before
 * this function returns; no pointer is retained by Rust. */
typedef struct LogiksmithRuntimeDocument {
    const LogiksmithRuntimeConfig *config;
    uint64_t document_revision;
    uint64_t structural_revision;
} LogiksmithRuntimeDocument;

typedef struct LogiksmithRuntimeDocumentBytes {
    const uint8_t *bytes;
    size_t len;
    uint64_t document_revision;
    uint64_t structural_revision;
} LogiksmithRuntimeDocumentBytes;

typedef struct LogiksmithInputEvent {
    const uint8_t *block_id;
    size_t block_id_len;
    const uint8_t *endpoint;
    size_t endpoint_len;
    LogiksmithValue value;
    /* Metadata is validated and retained at the boundary for host routing.
     * group_address is a non-zero 15-bit KNX group address. The portable core
     * otherwise intentionally does not interpret KNX addresses. */
    uint16_t source_address;
    uint16_t group_address;
    uint64_t monotonic_ms;
} LogiksmithInputEvent;

typedef struct LogiksmithEffect {
    uint8_t block_id[LOGIKSMITH_MAX_IDENTIFIER_BYTES];
    uint16_t block_id_len;
    uint8_t endpoint[LOGIKSMITH_MAX_IDENTIFIER_BYTES];
    uint16_t endpoint_len;
    LogiksmithValue value;
} LogiksmithEffect;

typedef struct LogiksmithActivation {
    const uint8_t *block_id;
    size_t block_id_len;
    const uint8_t *source;
    size_t source_len;
    uint8_t has_source;
    uint8_t enabled;
    uint8_t has_enabled;
    uint8_t reserved;
    uint64_t expected_document_revision;
    uint64_t expected_structural_revision;
    uint64_t expected_block_revision;
} LogiksmithActivation;

typedef struct LogiksmithActivationResult {
    uint8_t block_id[LOGIKSMITH_MAX_IDENTIFIER_BYTES];
    uint16_t block_id_len;
    uint16_t reserved;
    uint64_t logic_revision;
    uint64_t document_revision;
    uint64_t structural_revision;
    uint8_t enabled;
    uint8_t source_changed;
    uint8_t enabled_changed;
    uint8_t reserved2;
    size_t cancelled_timer_count;
} LogiksmithActivationResult;

typedef struct LogiksmithTimer {
    uint8_t name[LOGIKSMITH_MAX_IDENTIFIER_BYTES];
    uint16_t name_len;
    uint16_t reserved;
    uint64_t scheduled_at;
    uint64_t due_at;
    uint64_t scheduled_logic_revision;
} LogiksmithTimer;

typedef struct LogiksmithInputProjection {
    uint8_t endpoint[LOGIKSMITH_MAX_IDENTIFIER_BYTES];
    uint16_t endpoint_len;
    uint8_t valid;
    uint8_t reserved;
    LogiksmithValue value;
    uint64_t age_ms;
} LogiksmithInputProjection;

typedef struct LogiksmithBlockProjection {
    uint8_t block_id[LOGIKSMITH_MAX_IDENTIFIER_BYTES];
    uint16_t block_id_len;
    uint8_t health;
    uint8_t enabled;
    uint64_t logic_revision;
    uint32_t consecutive_failures;
    size_t input_count;
    size_t timer_count;
    LogiksmithInputProjection inputs[LOGIKSMITH_MAX_INPUTS_PER_BLOCK];
    LogiksmithTimer timers[LOGIKSMITH_MAX_TIMERS_PER_BLOCK];
} LogiksmithBlockProjection;

typedef struct LogiksmithRuntimeProjection {
    uint64_t document_revision;
    uint64_t structural_revision;
    uint64_t last_accepted_ms;
    uint8_t has_last_accepted;
    uint8_t reserved[7];
    size_t block_count;
    size_t signal_count;
    size_t signal_bindings;
    size_t logic_source_bytes;
    size_t state_entries;
    size_t state_bytes;
    size_t pending_timers;
} LogiksmithRuntimeProjection;

#ifdef __cplusplus
static_assert(sizeof(LogiksmithValue) == 12, "unexpected LogiksmithValue ABI layout");
static_assert(sizeof(LogiksmithEffect) == 144, "unexpected LogiksmithEffect ABI layout");
#else
_Static_assert(sizeof(LogiksmithValue) == 12, "unexpected LogiksmithValue ABI layout");
_Static_assert(sizeof(LogiksmithEffect) == 144, "unexpected LogiksmithEffect ABI layout");
#endif

typedef struct LogiksmithRuntime LogiksmithRuntime;

uint32_t logiksmith_abi_version(void);
uint8_t logiksmith_persistent_state_supported(void);

int32_t logiksmith_runtime_create_from_document(
    const LogiksmithRuntimeDocument *document,
    LogiksmithRuntime **out_runtime);

int32_t logiksmith_runtime_create_from_toml(
    const LogiksmithRuntimeDocumentBytes *document,
    LogiksmithRuntime **out_runtime);

int32_t logiksmith_runtime_create(
    const LogiksmithRuntimeConfig *config,
    LogiksmithRuntime **out_runtime);

int32_t logiksmith_runtime_destroy(LogiksmithRuntime *runtime);

/*
 * Process one triggering input. On success, [0, *written) contains effects.
 * If the buffer is too small, no runtime state is committed and *written is
 * set to the required number of records. A NULL effects pointer is accepted
 * only when capacity is zero.
 */
int32_t logiksmith_runtime_process_input(
    LogiksmithRuntime *runtime,
    const LogiksmithInputEvent *event,
    LogiksmithEffect *effects,
    size_t capacity,
    size_t *written);

int32_t logiksmith_runtime_process_due_timer(
    LogiksmithRuntime *runtime,
    uint64_t monotonic_ms,
    LogiksmithEffect *effects,
    size_t capacity,
    size_t *written);

int32_t logiksmith_runtime_validate_source(
    const uint8_t *source,
    size_t source_len,
    uint64_t *revision);

int32_t logiksmith_runtime_simulate_input(
    LogiksmithRuntime *runtime,
    const LogiksmithInputEvent *event,
    const uint8_t *source,
    size_t source_len,
    LogiksmithEffect *effects,
    size_t capacity,
    size_t *written);

int32_t logiksmith_runtime_simulate_timer(
    LogiksmithRuntime *runtime,
    const uint8_t *block_id,
    size_t block_id_len,
    const uint8_t *timer,
    size_t timer_len,
    uint64_t fired_at_ms,
    const uint8_t *source,
    size_t source_len,
    LogiksmithEffect *effects,
    size_t capacity,
    size_t *written);

int32_t logiksmith_runtime_activate(
    LogiksmithRuntime *runtime,
    const LogiksmithActivation *activation,
    LogiksmithActivationResult *result);

int32_t logiksmith_runtime_set_enabled(
    LogiksmithRuntime *runtime,
    const uint8_t *block_id,
    size_t block_id_len,
    uint8_t enabled,
    uint64_t expected_document_revision,
    uint64_t expected_structural_revision,
    uint64_t expected_block_revision,
    LogiksmithActivationResult *result);

int32_t logiksmith_runtime_resume(
    LogiksmithRuntime *runtime,
    const uint8_t *block_id,
    size_t block_id_len,
    uint64_t expected_document_revision,
    uint64_t expected_structural_revision,
    uint64_t expected_block_revision);

int32_t logiksmith_runtime_project(
    const LogiksmithRuntime *runtime,
    LogiksmithRuntimeProjection *result);

/* `written` is 1 on success. With a null result, the operation returns
 * LOGIKSMITH_STATUS_BUFFER_TOO_SMALL and writes the required record count. */
int32_t logiksmith_runtime_project_block(
    const LogiksmithRuntime *runtime,
    const uint8_t *block_id,
    size_t block_id_len,
    LogiksmithBlockProjection *result,
    size_t *written);

#ifdef __cplusplus
}
#endif

#endif /* LOGIKSMITH_EMBEDDED_ABI_H */
