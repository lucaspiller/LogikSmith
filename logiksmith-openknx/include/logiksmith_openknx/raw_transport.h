#pragma once

#include <stdint.h>

#include "logiksmith_openknx/raw_binding_router.h"

namespace logiksmith {
namespace openknx {

using RawGroupObserver = void (*)(void* context,
                                  uint16_t source_address,
                                  uint16_t destination_address,
                                  const uint8_t* payload,
                                  uint8_t payload_size);

// These two narrow symbols are supplied by the OpenKNX stack patch. Keeping
// them as a C ABI makes the host compile against the pinned stack while still
// allowing the native routing seam to be tested without OpenKNX headers.
bool register_raw_group_observer(void* context, RawGroupObserver observer);
bool send_raw_group_value(uint16_t destination_address,
                          const uint8_t* payload,
                          uint8_t payload_size);

class OpenKnxRawSender final : public RawGroupSender {
  public:
    bool send_group_value(uint16_t destination_address,
                          const uint8_t* payload,
                          uint8_t payload_size) override {
        return send_raw_group_value(destination_address, payload, payload_size);
    }
};

} // namespace openknx
} // namespace logiksmith
