#include "logiksmith_openknx/logic_smith_module.h"

#include <Arduino.h>

#include "logiksmith_openknx/default_bindings.h"
#include "logiksmith_openknx/raw_transport.h"

namespace logiksmith {
namespace openknx {

LogicSmithModule logicSmithModule;

LogicSmithModule::LogicSmithModule() : _router(_bindings) {
    (void)load_default_bindings(_bindings);
}

const std::string LogicSmithModule::name() {
    return "LogicSmith";
}

const std::string LogicSmithModule::version() {
    return "0.1.0-m14";
}

uint16_t LogicSmithModule::flashSize() {
    // M14 reserves no OpenKNX flash record yet. Binding persistence is kept as
    // a checked-in LittleFS scaffold until the M15 web/config store exists.
    return 0;
}

void LogicSmithModule::setup() {
    if (_abi_processor.start()) {
        _processor = &_abi_processor;
    }
    _raw_hook_registered = register_raw_group_observer(this, &LogicSmithModule::on_raw_group);
}

void LogicSmithModule::loop() {
    // Keep the per-loop work bounded so a burst of raw KNX traffic cannot
    // starve other OpenKNX modules (for example a motion/switch application).
    if (_processor != nullptr) {
        InputEvent event;
        for (size_t count = 0; count < 4 && _router.pop_input(event); ++count) {
            _processor->process_input(event, _router);
        }
        _processor->tick(millis(), _router);
    }
    (void)_router.drain_outputs(_sender, 4);
}

void LogicSmithModule::on_raw_group(void* context,
                                    uint16_t source_address,
                                    uint16_t destination_address,
                                    const uint8_t* payload,
                                    uint8_t payload_size) {
    if (context == nullptr) {
        return;
    }
    static_cast<LogicSmithModule*>(context)->ingest_raw_group(
        source_address, destination_address, payload, payload_size);
}

void LogicSmithModule::ingest_raw_group(uint16_t source_address,
                                        uint16_t destination_address,
                                        const uint8_t* payload,
                                        uint8_t payload_size) {
    RawGroupTelegram telegram;
    telegram.source_address = source_address;
    telegram.destination_address = destination_address;
    telegram.received_at_ms = millis();
    // Never dereference a malformed pointer from the stack callback. The
    // router records a null/empty payload as malformed and increments its
    // diagnostics counter.
    if (payload != nullptr && payload_size <= kMaxTelegramPayload) {
        telegram.payload_size = payload_size;
        memcpy(telegram.payload, payload, payload_size);
    } else {
        telegram.payload_size = 0;
    }
    (void)_router.ingest(telegram);
}

} // namespace openknx
} // namespace logiksmith
