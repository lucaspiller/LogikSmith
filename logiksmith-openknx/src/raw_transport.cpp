#include "logiksmith_openknx/raw_transport.h"

#include "logiksmith_openknx/raw_binding_router.h"

#if defined(LOGIKSMITH_OPENKNX_DEVICE)
#include <knx.h>
#endif

namespace {

using Observer = logiksmith::openknx::RawGroupObserver;

#if defined(LOGIKSMITH_OPENKNX_DEVICE)
// These wrappers are the only place where the host knows the concrete
// OpenKNX BAU device. The stack patch adds the two corresponding BAU methods;
// all routing and tests stay independent of OpenKNX headers.
extern "C" bool logiksmith_openknx_register_raw_group_observer(void* context,
                                                                Observer observer) {
    knx.bau().setRawGroupObserver(context, observer);
    return true;
}

extern "C" bool logiksmith_openknx_send_raw_group_value(uint16_t destination_address,
                                                         const uint8_t* payload,
                                                         uint8_t payload_size) {
    return knx.bau().rawGroupValueWrite(destination_address, payload, payload_size);
}
#else
// Native routing tests do not link OpenKNX. Weak declarations leave the
// transport adapter unavailable while preserving a useful host seam.
extern "C" bool logiksmith_openknx_register_raw_group_observer(void* context,
                                                                Observer observer)
    __attribute__((weak));
extern "C" bool logiksmith_openknx_send_raw_group_value(uint16_t destination_address,
                                                         const uint8_t* payload,
                                                         uint8_t payload_size)
    __attribute__((weak));
#endif

} // namespace

namespace logiksmith {
namespace openknx {

bool register_raw_group_observer(void* context, RawGroupObserver observer) {
    if (logiksmith_openknx_register_raw_group_observer == nullptr || observer == nullptr) {
        return false;
    }
    return logiksmith_openknx_register_raw_group_observer(context, observer);
}

bool send_raw_group_value(uint16_t destination_address,
                          const uint8_t* payload,
                          uint8_t payload_size) {
    if (logiksmith_openknx_send_raw_group_value == nullptr || payload == nullptr ||
        payload_size == 0) {
        return false;
    }
    return logiksmith_openknx_send_raw_group_value(destination_address, payload,
                                                   payload_size);
}

} // namespace openknx
} // namespace logiksmith
