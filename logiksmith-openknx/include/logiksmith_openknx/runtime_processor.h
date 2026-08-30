#pragma once

#include <stdint.h>

#include "logiksmith_openknx/raw_binding_router.h"

namespace logiksmith {
namespace openknx {

// The Rust ABI adapter implements this small seam. It is deliberately a
// callback interface rather than a dependency from the portable router on
// Rust, Lua, or OpenKNX.
class RuntimeProcessor {
  public:
    virtual ~RuntimeProcessor() = default;

    // Called from the OpenKNX loop context, never from the KNX ingress hook.
    // Implementations may enqueue validated output effects on the router.
    virtual void process_input(const InputEvent& event,
                               RawBindingRouter& router) = 0;

    // Gives the runtime a bounded periodic opportunity to advance timers.
    virtual void tick(uint32_t now_ms, RawBindingRouter& router) {
        (void)now_ms;
        (void)router;
    }
};

// Until the Rust ABI adapter is installed, the device remains a valid
// OpenKNX module with an explicit no-op runtime. It drains input records and
// makes the missing integration visible through the host wiring instead of
// silently retaining an unbounded queue.
class DisabledRuntimeProcessor final : public RuntimeProcessor {
  public:
    void process_input(const InputEvent& event, RawBindingRouter& router) override {
        (void)event;
        (void)router;
    }
};

} // namespace openknx
} // namespace logiksmith
