#include "logiksmith_openknx/abi_runtime_processor.h"

#include <string.h>

// Native host tests can exercise the boundary without a Rust archive, but a
// release image must pull the archive and fail at link time if it is absent.
#if defined(LOGIKSMITH_REQUIRE_ABI_RUNTIME)
#define LOGIKSMITH_ABI_WEAK
#else
#define LOGIKSMITH_ABI_WEAK __attribute__((weak))
#endif
extern "C" uint32_t logiksmith_abi_version(void) LOGIKSMITH_ABI_WEAK;
extern "C" int32_t logiksmith_runtime_create(
    const LogiksmithRuntimeConfig* config,
    LogiksmithRuntime** out_runtime) LOGIKSMITH_ABI_WEAK;
extern "C" int32_t logiksmith_runtime_destroy(LogiksmithRuntime* runtime)
    LOGIKSMITH_ABI_WEAK;
extern "C" int32_t logiksmith_runtime_process_input(
    LogiksmithRuntime* runtime,
    const LogiksmithInputEvent* event,
    LogiksmithEffect* effects,
    size_t capacity,
    size_t* written) LOGIKSMITH_ABI_WEAK;
#undef LOGIKSMITH_ABI_WEAK

namespace logiksmith {
namespace openknx {

AbiRuntimeProcessor::~AbiRuntimeProcessor() {
    stop();
}

bool AbiRuntimeProcessor::start() {
    if (_runtime != nullptr || logiksmith_abi_version == nullptr ||
        logiksmith_runtime_create == nullptr ||
        logiksmith_runtime_process_input == nullptr ||
        logiksmith_runtime_destroy == nullptr ||
        logiksmith_abi_version() != LOGIKSMITH_ABI_VERSION) {
        return false;
    }

    static const uint8_t trigger_name[] = "trigger";
    static const uint8_t light_name[] = "light";
    static const uint8_t block_name[] = "main";
    static const uint8_t source[] =
        "function handle(event)\n"
        "  if event.type == 'input' and event.input == 'trigger' and event.value == true then\n"
        "    return { outputs = { light = true } }\n"
        "  end\n"
        "  return {}\n"
        "end\n";
    const LogiksmithEndpointConfig endpoints[] = {
        {trigger_name, sizeof(trigger_name) - 1, LOGIKSMITH_ENDPOINT_INPUT, {0, 0, 0}, 1,
         1},
        {light_name, sizeof(light_name) - 1, LOGIKSMITH_ENDPOINT_OUTPUT, {0, 0, 0}, 1,
         1},
    };
    const LogiksmithBlockConfig block = {
        block_name,
        sizeof(block_name) - 1,
        source,
        sizeof(source) - 1,
        endpoints,
        sizeof(endpoints) / sizeof(endpoints[0]),
    };
    const LogiksmithRuntimeConfig config = {&block, 1};

    LogiksmithRuntime* runtime = nullptr;
    if (logiksmith_runtime_create(&config, &runtime) != LOGIKSMITH_STATUS_OK) {
        return false;
    }
    _runtime = runtime;
    return true;
}

void AbiRuntimeProcessor::shutdown() {
    stop();
}

void AbiRuntimeProcessor::stop() {
    if (_runtime != nullptr && logiksmith_runtime_destroy != nullptr) {
        (void)logiksmith_runtime_destroy(_runtime);
    }
    _runtime = nullptr;
}

bool AbiRuntimeProcessor::copy_endpoint(const uint8_t* bytes,
                                        uint16_t length,
                                        EndpointId& endpoint) {
    if (bytes == nullptr || length == 0 || length > kMaxEndpointName) {
        return false;
    }
    char value[kMaxEndpointName + 1] = {};
    memcpy(value, bytes, length);
    return endpoint.assign(value);
}

bool AbiRuntimeProcessor::is_bool_value(const LogiksmithValue& value) {
    return value.dpt_major == 1 && value.dpt_subtype == 1 &&
           value.kind == LOGIKSMITH_VALUE_BOOL &&
           (value.scalar == 0 || value.scalar == 1) && value.reserved == 0 &&
           value.reserved2 == 0;
}

void AbiRuntimeProcessor::process_input(const InputEvent& event,
                                        RawBindingRouter& router) {
    if (_runtime == nullptr || !(event.dpt == kDptBool) || event.payload_size != 1) {
        return;
    }

    const uint8_t block_id[] = "main";
    LogiksmithInputEvent input = {};
    input.block_id = block_id;
    input.block_id_len = sizeof(block_id) - 1;
    input.endpoint = reinterpret_cast<const uint8_t*>(event.endpoint.c_str());
    input.endpoint_len = event.endpoint.size();
    input.value = {1, 1, LOGIKSMITH_VALUE_BOOL, 0, 0, event.bool_value() ? 1 : 0};
    input.source_address = event.source_address;
    input.group_address = event.group_address;
    input.monotonic_ms = event.received_at_ms;

    size_t written = 0;
    const int32_t status = logiksmith_runtime_process_input(
        _runtime, &input, _effects, kEffectCapacity, &written);
    if (status != LOGIKSMITH_STATUS_OK || written > kEffectCapacity) {
        return;
    }

    for (size_t index = 0; index < written; ++index) {
        const LogiksmithEffect& effect = _effects[index];
        if (effect.block_id_len != 4 ||
            memcmp(effect.block_id, "main", effect.block_id_len) != 0 ||
            !is_bool_value(effect.value)) {
            continue;
        }
        EndpointId endpoint;
        if (!copy_endpoint(effect.endpoint, effect.endpoint_len, endpoint)) {
            continue;
        }
        (void)router.enqueue_bool_output(endpoint, effect.value.scalar != 0);
    }
}

} // namespace openknx
} // namespace logiksmith
