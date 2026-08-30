#pragma once

#include <stddef.h>
#include <stdint.h>

#include "logiksmith_embedded_abi.h"
#include "logiksmith_openknx/runtime_processor.h"

namespace logiksmith {
namespace openknx {

// A small default block keeps the M14 firmware executable when the Rust
// static library is not linked. The weak ABI symbols turn that absence into a
// clean fallback instead of a link error; a later portable-core build can
// provide the same symbols without changing this host boundary.
class AbiRuntimeProcessor final : public RuntimeProcessor {
  public:
    AbiRuntimeProcessor() = default;
    ~AbiRuntimeProcessor() override;

    bool start();
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
