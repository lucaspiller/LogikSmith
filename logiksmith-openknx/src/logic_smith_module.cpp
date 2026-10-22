#include "logiksmith_openknx/logic_smith_module.h"

#include <ArduinoOTA.h>
#include <Arduino.h>
#include <WiFi.h>

#include "logiksmith_openknx/default_bindings.h"
#include "logiksmith_openknx/raw_transport.h"

#if __has_include("../../config/wifi.h")
#include "../../config/wifi.h"
#else
#include "../../config/wifi.h.example"
#endif

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
    return "0.1.0-m15";
}

uint16_t LogicSmithModule::flashSize() {
    // M15 keeps persistence behind the native-testable store seam. The
    // concrete LittleFS adapter and reserved-record accounting remain host
    // integration work, so report no OpenKNX flash record here.
    return 0;
}

void LogicSmithModule::setup() {
    _runtime_start_failed = false;
    if (_abi_processor.start()) {
        _processor = &_abi_processor;
        Serial.println("[LogicSmith] processor: embedded Rust ABI");
    } else {
#if defined(LOGIKSMITH_REQUIRE_ABI_RUNTIME)
        // Release images fail closed. The post-link guard should catch this
        // before flashing; this branch also keeps an unexpected ABI mismatch
        // from silently running the disabled processor on a manually-built
        // image.
        _processor = nullptr;
        _runtime_start_failed = true;
        Serial.println("[LogicSmith] fatal: embedded runtime ABI is unavailable");
#else
        _processor = &_disabled_processor;
#endif
    }
    _raw_hook_registered = register_raw_group_observer(this, &LogicSmithModule::on_raw_group);
    setup_wifi();
}

void LogicSmithModule::processBeforeRestart() {
    _abi_processor.shutdown();
    _processor = nullptr;
    _runtime_start_failed = true;
}

void LogicSmithModule::loop() {
    if (_runtime_start_failed) {
        return;
    }
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
    // Management is deliberately the final bounded slice. KNX ingress, due
    // timers, and output draining retain priority on every pass.
    if (_management_server != nullptr) {
        (void)_management_server->process_one();
    }
    process_wifi_ota();
}

void LogicSmithModule::setup_wifi() {
    if (LOGIKSMITH_WIFI_SSID[0] == '\0') {
        Serial.println("[LogikSmith] Wi-Fi disabled; set config/wifi.h");
        return;
    }

    WiFi.mode(WIFI_STA);
    WiFi.setAutoReconnect(true);
    WiFi.begin(LOGIKSMITH_WIFI_SSID, LOGIKSMITH_WIFI_PASSWORD);
    _wifi_enabled = true;
    Serial.println("[LogikSmith] connecting to configured Wi-Fi");

    if (LOGIKSMITH_OTA_PASSWORD[0] == '\0') {
        Serial.println("[LogikSmith] OTA disabled; set LOGIKSMITH_OTA_PASSWORD in config/wifi.h");
    }
}

void LogicSmithModule::process_wifi_ota() {
    if (!_wifi_enabled) {
        return;
    }

    if (WiFi.status() != WL_CONNECTED) {
        if (_wifi_connected) {
            Serial.println("[LogikSmith] Wi-Fi disconnected");
            _wifi_connected = false;
        }
        return;
    }

    if (!_wifi_connected) {
        Serial.printf("[LogikSmith] Wi-Fi connected: %s\n", WiFi.localIP().toString().c_str());
        _wifi_connected = true;
    }

    if (!_ota_started && LOGIKSMITH_OTA_PASSWORD[0] != '\0') {
        ArduinoOTA.setHostname("logiksmith-openknx");
        ArduinoOTA.setPassword(LOGIKSMITH_OTA_PASSWORD);
        ArduinoOTA.onStart([]() { Serial.println("[LogikSmith] OTA update started"); });
        ArduinoOTA.onEnd([]() { Serial.println("[LogikSmith] OTA update complete; rebooting"); });
        ArduinoOTA.onError([](ota_error_t error) {
            Serial.printf("[LogikSmith] OTA update failed, error %u\n", static_cast<unsigned>(error));
        });
        ArduinoOTA.begin();
        _ota_started = true;
        Serial.println("[LogikSmith] OTA ready on port 3232");
    }

    if (_ota_started) {
        ArduinoOTA.handle();
    }
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
