#ifndef LOGIKSMITH_EMBEDDED_ABI_H
#define LOGIKSMITH_EMBEDDED_ABI_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

#define LOGIKSMITH_ABI_VERSION 1u
#define LOGIKSMITH_MAX_IDENTIFIER_BYTES 64u
#define LOGIKSMITH_MAX_SOURCE_BYTES (64u * 1024u)
#define LOGIKSMITH_MAX_BLOCKS 64u
#define LOGIKSMITH_MAX_ENDPOINTS_PER_BLOCK 64u
#define LOGIKSMITH_MAX_EFFECTS 256u

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

#ifdef __cplusplus
static_assert(sizeof(LogiksmithValue) == 12, "unexpected LogiksmithValue ABI layout");
static_assert(sizeof(LogiksmithEffect) == 144, "unexpected LogiksmithEffect ABI layout");
#else
_Static_assert(sizeof(LogiksmithValue) == 12, "unexpected LogiksmithValue ABI layout");
_Static_assert(sizeof(LogiksmithEffect) == 144, "unexpected LogiksmithEffect ABI layout");
#endif

typedef struct LogiksmithRuntime LogiksmithRuntime;

uint32_t logiksmith_abi_version(void);

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

#ifdef __cplusplus
}
#endif

#endif /* LOGIKSMITH_EMBEDDED_ABI_H */
