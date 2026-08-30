#include "logiksmith_openknx/raw_binding_router.h"

namespace logiksmith {
namespace openknx {

namespace {

bool payload_size_matches(DptId dpt, uint8_t size) {
    if (dpt == kDptBool || dpt == kDptPercent) {
        return size == 1;
    }
    if (dpt == kDptTemperature) {
        return size == 2;
    }
    return false;
}

bool same_output_endpoint(const Binding& left, const Binding& right) {
    return left.direction == BindingDirection::Output &&
           right.direction == BindingDirection::Output &&
           left.endpoint == right.endpoint;
}

} // namespace

bool BindingTable::valid_dpt(DptId dpt) {
    return dpt == kDptBool || dpt == kDptPercent || dpt == kDptTemperature;
}

BindingTableError BindingTable::replace(const Binding* bindings, size_t count) {
    if (count > kMaxBindings) {
        return BindingTableError::TooManyBindings;
    }
    if (count > 0 && bindings == nullptr) {
        return BindingTableError::NullBindings;
    }

    // Validate the incoming table before replacing the active copy. This
    // keeps a malformed M15/web update from partially changing routing.
    for (size_t index = 0; index < count; ++index) {
        const Binding& binding = bindings[index];
        if (binding.group_address == 0 || binding.group_address > 0x7FFF) {
            return BindingTableError::InvalidGroupAddress;
        }
        if (binding.endpoint.empty() || binding.endpoint.size() > kMaxEndpointName) {
            return BindingTableError::InvalidEndpoint;
        }
        if (!valid_dpt(binding.dpt)) {
            return BindingTableError::UnsupportedDpt;
        }
        for (size_t previous = 0; previous < index; ++previous) {
            const Binding& old = bindings[previous];
            if (same_output_endpoint(old, binding)) {
                return BindingTableError::DuplicateOutput;
            }
            if (old.direction == binding.direction &&
                old.group_address == binding.group_address) {
                return BindingTableError::DuplicateGroupAddress;
            }
        }
    }

    for (size_t index = 0; index < count; ++index) {
        _bindings[index] = bindings[index];
    }
    _size = count;
    return BindingTableError::None;
}

const Binding* BindingTable::find_input(uint16_t group_address) const {
    for (size_t index = 0; index < _size; ++index) {
        const Binding& binding = _bindings[index];
        if (binding.direction == BindingDirection::Input &&
            binding.group_address == group_address) {
            return &binding;
        }
    }
    return nullptr;
}

const Binding* BindingTable::find_output(const EndpointId& endpoint, DptId dpt) const {
    for (size_t index = 0; index < _size; ++index) {
        const Binding& binding = _bindings[index];
        if (binding.direction == BindingDirection::Output &&
            binding.endpoint == endpoint && binding.dpt == dpt) {
            return &binding;
        }
    }
    return nullptr;
}

IngressResult RawBindingRouter::ingest(const RawGroupTelegram& telegram) {
    _telegrams_seen.fetch_add(1, std::memory_order_relaxed);

    if (telegram.destination_address == 0 ||
        telegram.destination_address > 0x7FFF ||
        telegram.payload_size == 0 ||
        telegram.payload_size > kMaxTelegramPayload) {
        _telegrams_malformed.fetch_add(1, std::memory_order_relaxed);
        return IngressResult::Malformed;
    }

    const Binding* binding = _bindings.find_input(telegram.destination_address);
    if (binding == nullptr) {
        _telegrams_ignored_unbound.fetch_add(1, std::memory_order_relaxed);
        return IngressResult::IgnoredUnbound;
    }
    if (!payload_size_matches(binding->dpt, telegram.payload_size)) {
        _telegrams_malformed.fetch_add(1, std::memory_order_relaxed);
        return IngressResult::Malformed;
    }

    InputEvent event;
    event.endpoint = binding->endpoint;
    event.dpt = binding->dpt;
    event.source_address = telegram.source_address;
    event.group_address = telegram.destination_address;
    event.payload_size = telegram.payload_size;
    event.received_at_ms = telegram.received_at_ms;
    memcpy(event.payload, telegram.payload, telegram.payload_size);

    if (!_inputs.push(event)) {
        _input_queue_full.fetch_add(1, std::memory_order_relaxed);
        return IngressResult::QueueFull;
    }
    _telegrams_enqueued.fetch_add(1, std::memory_order_relaxed);
    return IngressResult::Enqueued;
}

OutputResult RawBindingRouter::enqueue_output(const EndpointId& endpoint,
                                              DptId dpt,
                                              const uint8_t* payload,
                                              uint8_t payload_size) {
    if (payload == nullptr || payload_size == 0 ||
        payload_size > kMaxTelegramPayload || !payload_size_matches(dpt, payload_size)) {
        return OutputResult::Malformed;
    }

    const Binding* binding = _bindings.find_output(endpoint, dpt);
    if (binding == nullptr) {
        _outputs_unbound.fetch_add(1, std::memory_order_relaxed);
        return OutputResult::Unbound;
    }

    RawGroupValue value;
    value.destination_address = binding->group_address;
    value.payload_size = payload_size;
    memcpy(value.payload, payload, payload_size);
    if (!_outputs.push(value)) {
        _output_queue_full.fetch_add(1, std::memory_order_relaxed);
        return OutputResult::QueueFull;
    }
    _outputs_enqueued.fetch_add(1, std::memory_order_relaxed);
    return OutputResult::Enqueued;
}

OutputResult RawBindingRouter::enqueue_bool_output(const EndpointId& endpoint, bool value) {
    const uint8_t payload = value ? 1 : 0;
    return enqueue_output(endpoint, kDptBool, &payload, 1);
}

size_t RawBindingRouter::drain_outputs(RawGroupSender& sender, size_t limit) {
    size_t processed = 0;
    RawGroupValue value;
    while (processed < limit && _outputs.pop(value)) {
        ++processed;
        if (sender.send_group_value(value.destination_address, value.payload,
                                    value.payload_size)) {
            _outputs_sent.fetch_add(1, std::memory_order_relaxed);
        } else {
            _outputs_failed.fetch_add(1, std::memory_order_relaxed);
        }
    }
    return processed;
}

RouterStats RawBindingRouter::stats() const {
    RouterStats result;
    result.telegrams_seen = _telegrams_seen.load(std::memory_order_relaxed);
    result.telegrams_enqueued = _telegrams_enqueued.load(std::memory_order_relaxed);
    result.telegrams_ignored_unbound = _telegrams_ignored_unbound.load(std::memory_order_relaxed);
    result.telegrams_malformed = _telegrams_malformed.load(std::memory_order_relaxed);
    result.input_queue_full = _input_queue_full.load(std::memory_order_relaxed);
    result.outputs_enqueued = _outputs_enqueued.load(std::memory_order_relaxed);
    result.outputs_unbound = _outputs_unbound.load(std::memory_order_relaxed);
    result.output_queue_full = _output_queue_full.load(std::memory_order_relaxed);
    result.outputs_sent = _outputs_sent.load(std::memory_order_relaxed);
    result.outputs_failed = _outputs_failed.load(std::memory_order_relaxed);
    return result;
}

} // namespace openknx
} // namespace logiksmith
