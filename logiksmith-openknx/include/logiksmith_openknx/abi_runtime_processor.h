#pragma once

#include <stddef.h>
#include <stdint.h>

#include "logiksmith_embedded_abi.h"
#include "logiksmith_openknx/runtime_processor.h"

namespace logiksmith {
namespace openknx {

// The default block gives the M14 image a deterministic executable path. In
// native tests the ABI symbols may be weak, while the release build promotes
// them to strong references so an absent Rust archive cannot become a silent
// disabled-runtime firmware image.
class AbiRuntimeProcessor final : public RuntimeProcessor {
  public:
    AbiRuntimeProcessor() = default;
    ~AbiRuntimeProcessor() override;

    bool start();
    void shutdown();
    bool started() const { return _runtime != nullptr; }

    void process_input(const InputEvent& event, RawBindingRouter& router) override;

  private:
    static constexpr size_t kEffectCapacity = 8;

    void stop();
    static bool copy_endpoint(const uint8_t* bytes, uint16_t length, EndpointId& endpoint);
    static bool is_bool_value(const LogiksmithValue& value);

    LogiksmithRuntime* _runtime = nullptr;
    LogiksmithEffect _effects[kEffectCapacity] = {};
};

} // namespace openknx
} // namespace logiksmith
