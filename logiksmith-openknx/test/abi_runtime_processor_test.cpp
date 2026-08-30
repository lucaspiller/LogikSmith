#include "logiksmith_openknx/abi_runtime_processor.h"
#include "logiksmith_openknx/default_bindings.h"

#include <assert.h>
#include <string.h>

using namespace logiksmith::openknx;

struct LogiksmithRuntime {
    unsigned process_calls = 0;
};

extern "C" uint32_t logiksmith_abi_version(void) {
    return LOGIKSMITH_ABI_VERSION;
}

extern "C" int32_t logiksmith_runtime_create(const LogiksmithRuntimeConfig* config,
                                               LogiksmithRuntime** out_runtime) {
    assert(config != nullptr);
    assert(config->block_count == 1);
    assert(out_runtime != nullptr);
    *out_runtime = new LogiksmithRuntime();
    return LOGIKSMITH_STATUS_OK;
}

extern "C" int32_t logiksmith_runtime_destroy(LogiksmithRuntime* runtime) {
    delete runtime;
    return LOGIKSMITH_STATUS_OK;
}

extern "C" int32_t logiksmith_runtime_process_input(LogiksmithRuntime* runtime,
                                                       const LogiksmithInputEvent* event,
                                                       LogiksmithEffect* effects,
                                                       size_t capacity,
                                                       size_t* written) {
    assert(runtime != nullptr);
    assert(event != nullptr);
    assert(effects != nullptr);
    assert(capacity > 0);
    ++runtime->process_calls;
    memset(effects, 0, sizeof(*effects));
    memcpy(effects[0].block_id, "main", 4);
    effects[0].block_id_len = 4;
    memcpy(effects[0].endpoint, "light", 5);
    effects[0].endpoint_len = 5;
    effects[0].value = {1, 1, LOGIKSMITH_VALUE_BOOL, 0, 0, event->value.scalar};
    *written = 1;
    return LOGIKSMITH_STATUS_OK;
}

class RecordingSender final : public RawGroupSender {
  public:
    bool send_group_value(uint16_t destination_address,
                          const uint8_t* payload,
                          uint8_t payload_size) override {
        destination = destination_address;
        value = payload_size == 1 ? payload[0] : 0xff;
        return true;
    }

    uint16_t destination = 0;
    uint8_t value = 0;
};

int main() {
    BindingTable bindings;
    assert(load_default_bindings(bindings) == BindingTableError::None);
    RawBindingRouter router(bindings);

    AbiRuntimeProcessor processor;
    assert(processor.start());
    InputEvent event;
    assert(event.endpoint.assign("trigger"));
    event.dpt = kDptBool;
    event.source_address = 0x1201;
    event.group_address = kDefaultTriggerGroupAddress;
    event.payload[0] = 1;
    event.payload_size = 1;
    processor.process_input(event, router);

    RecordingSender sender;
    assert(router.drain_outputs(sender) == 1);
    assert(sender.destination == kDefaultLightGroupAddress);
    assert(sender.value == 1);
    return 0;
}
