#pragma once

#include "OpenKNX/Module.h"

#include "logiksmith_openknx/abi_runtime_processor.h"
#include "logiksmith_openknx/raw_binding_router.h"
#include "logiksmith_openknx/raw_transport.h"
#include "logiksmith_openknx/runtime_processor.h"

namespace logiksmith {
namespace openknx {

class LogicSmithModule final : public OpenKNX::Module {
  public:
    LogicSmithModule();
    ~LogicSmithModule() = default;

    const std::string name() override;
    const std::string version() override;
    uint16_t flashSize() override;

    void setup() override;
    void loop() override;

    void set_runtime_processor(RuntimeProcessor* processor) {
        _processor = processor == nullptr ? &_disabled_processor : processor;
    }
    BindingTableError replace_bindings(const Binding* bindings, size_t count) {
        return _bindings.replace(bindings, count);
    }
    BindingTable& bindings() { return _bindings; }
    RawBindingRouter& router() { return _router; }
    bool raw_hook_registered() const { return _raw_hook_registered; }

  private:
    static void on_raw_group(void* context,
                             uint16_t source_address,
                             uint16_t destination_address,
                             const uint8_t* payload,
                             uint8_t payload_size);
    void ingest_raw_group(uint16_t source_address,
                          uint16_t destination_address,
                          const uint8_t* payload,
                          uint8_t payload_size);

    BindingTable _bindings;
    RawBindingRouter _router;
    OpenKnxRawSender _sender;
    AbiRuntimeProcessor _abi_processor;
    DisabledRuntimeProcessor _disabled_processor;
    RuntimeProcessor* _processor = &_disabled_processor;
    bool _raw_hook_registered = false;
};

extern LogicSmithModule logicSmithModule;

} // namespace openknx
} // namespace logiksmith
