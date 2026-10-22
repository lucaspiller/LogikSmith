#pragma once

#include "OpenKNX/Module.h"

#include "logiksmith_openknx/abi_runtime_processor.h"
#include "logiksmith_openknx/management_server.h"
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
    void processBeforeRestart() override;

    void set_runtime_processor(RuntimeProcessor* processor) {
#if defined(LOGIKSMITH_REQUIRE_ABI_RUNTIME)
        // A release image must never be switched back to the disabled
        // processor through a test/configuration seam.
        if (processor != nullptr && processor != &_disabled_processor) {
            _processor = processor;
        }
#else
        _processor = processor == nullptr ? &_disabled_processor : processor;
#endif
    }
    BindingTableError replace_bindings(const Binding* bindings, size_t count) {
        return _bindings.replace(bindings, count);
    }
    BindingTable& bindings() { return _bindings; }
    RawBindingRouter& router() { return _router; }
    bool raw_hook_registered() const { return _raw_hook_registered; }
    bool runtime_processor_available() const {
        return _processor == &_abi_processor && _abi_processor.started();
    }
    bool runtime_start_failed() const { return _runtime_start_failed; }
    void set_management_server(ManagementServer* server) { _management_server = server; }

  private:
    static void on_raw_group(void* context,
                             uint16_t source_address,
                             uint16_t destination_address,
                             const uint8_t* payload,
                             uint8_t payload_size);
    void setup_wifi();
    void process_wifi_ota();
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
    ManagementServer* _management_server = nullptr;
    bool _raw_hook_registered = false;
    bool _runtime_start_failed = false;
    bool _wifi_enabled = false;
    bool _wifi_connected = false;
    bool _ota_started = false;
};

extern LogicSmithModule logicSmithModule;

} // namespace openknx
} // namespace logiksmith
