#include "logiksmith_openknx/default_bindings.h"
#include "logiksmith_openknx/runtime_processor.h"

#include <assert.h>
#include <string.h>

using namespace logiksmith::openknx;

namespace {

class RecordingSender final : public RawGroupSender {
  public:
    bool send_group_value(uint16_t destination_address,
                          const uint8_t* payload,
                          uint8_t payload_size) override {
        ++calls;
        destination = destination_address;
        size = payload_size;
        memcpy(bytes, payload, payload_size);
        return succeed;
    }

    bool succeed = true;
    unsigned calls = 0;
    uint16_t destination = 0;
    uint8_t bytes[kMaxTelegramPayload] = {};
    uint8_t size = 0;
};

Binding binding(const char* endpoint,
                uint16_t address,
                BindingDirection direction) {
    Binding result;
    result.group_address = address;
    assert(result.endpoint.assign(endpoint));
    result.dpt = kDptBool;
    result.direction = direction;
    return result;
}

RawGroupTelegram telegram(uint16_t address, uint8_t value) {
    RawGroupTelegram result;
    result.source_address = 0x1201;
    result.destination_address = address;
    result.payload[0] = value;
    result.payload_size = 1;
    result.received_at_ms = 42;
    return result;
}

void bound_input_is_decoded_without_ets_association() {
    Binding bindings[2] = {binding("trigger", 0x0101, BindingDirection::Input),
                           binding("light", 0x0102, BindingDirection::Output)};
    BindingTable table;
    assert(table.replace(bindings, 2) == BindingTableError::None);
    RawBindingRouter router(table);

    assert(router.ingest(telegram(0x0100, 1)) == IngressResult::IgnoredUnbound);
    assert(router.ingest(telegram(0x0101, 1)) == IngressResult::Enqueued);

    InputEvent event;
    assert(router.pop_input(event));
    assert(event.endpoint == bindings[0].endpoint);
    assert(event.dpt == kDptBool);
    assert(event.bool_value());
    assert(event.source_address == 0x1201);
    assert(event.received_at_ms == 42);

    const RouterStats stats = router.stats();
    assert(stats.telegrams_seen == 2);
    assert(stats.telegrams_ignored_unbound == 1);
    assert(stats.telegrams_enqueued == 1);
}

void output_effect_uses_current_binding_address() {
    Binding bindings[1] = {binding("light", 0x0102, BindingDirection::Output)};
    BindingTable table;
    assert(table.replace(bindings, 1) == BindingTableError::None);
    RawBindingRouter router(table);
    EndpointId light;
    assert(light.assign("light"));

    assert(router.enqueue_bool_output(light, true) == OutputResult::Enqueued);
    RecordingSender sender;
    assert(router.drain_outputs(sender) == 1);
    assert(sender.calls == 1);
    assert(sender.destination == 0x0102);
    assert(sender.size == 1);
    assert(sender.bytes[0] == 1);
}

void replacing_bindings_changes_routing_without_ets() {
    Binding old_binding = binding("trigger", 0x0101, BindingDirection::Input);
    BindingTable table;
    assert(table.replace(&old_binding, 1) == BindingTableError::None);
    RawBindingRouter router(table);
    assert(router.ingest(telegram(0x0101, 1)) == IngressResult::Enqueued);

    Binding new_binding = binding("trigger", 0x0201, BindingDirection::Input);
    assert(table.replace(&new_binding, 1) == BindingTableError::None);
    assert(router.ingest(telegram(0x0101, 1)) == IngressResult::IgnoredUnbound);
    assert(router.ingest(telegram(0x0201, 0)) == IngressResult::Enqueued);

    InputEvent first;
    InputEvent second;
    assert(router.pop_input(first));
    assert(router.pop_input(second));
    assert(first.group_address == 0x0101);
    assert(second.group_address == 0x0201);
    assert(!second.bool_value());
}

void queues_are_bounded_and_non_blocking() {
    Binding input = binding("trigger", 0x0101, BindingDirection::Input);
    BindingTable table;
    assert(table.replace(&input, 1) == BindingTableError::None);
    RawBindingRouter router(table);
    for (size_t count = 0; count < kInputQueueCapacity; ++count) {
        assert(router.ingest(telegram(0x0101, 1)) == IngressResult::Enqueued);
    }
    assert(router.ingest(telegram(0x0101, 1)) == IngressResult::QueueFull);
    assert(router.stats().input_queue_full == 1);
}

class DeterministicRuntime final : public RuntimeProcessor {
  public:
    void process_input(const InputEvent& event, RawBindingRouter& router) override {
        if (event.endpoint == trigger && event.dpt == kDptBool) {
            assert(router.enqueue_bool_output(light, event.bool_value()) == OutputResult::Enqueued);
        }
    }

    EndpointId trigger;
    EndpointId light;
};

void runtime_seam_carries_input_to_output() {
    Binding bindings[2] = {binding("trigger", 0x0101, BindingDirection::Input),
                           binding("light", 0x0102, BindingDirection::Output)};
    BindingTable table;
    assert(table.replace(bindings, 2) == BindingTableError::None);
    RawBindingRouter router(table);
    DeterministicRuntime runtime;
    assert(runtime.trigger.assign("trigger"));
    assert(runtime.light.assign("light"));
    assert(router.ingest(telegram(0x0101, 1)) == IngressResult::Enqueued);

    InputEvent event;
    assert(router.pop_input(event));
    runtime.process_input(event, router);

    RecordingSender sender;
    assert(router.drain_outputs(sender) == 1);
    assert(sender.destination == 0x0102);
    assert(sender.bytes[0] == 1);
}

} // namespace

int main() {
    bound_input_is_decoded_without_ets_association();
    output_effect_uses_current_binding_address();
    replacing_bindings_changes_routing_without_ets();
    queues_are_bounded_and_non_blocking();
    runtime_seam_carries_input_to_output();
    return 0;
}
