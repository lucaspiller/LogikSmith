#pragma once

#include <atomic>
#include <stddef.h>
#include <stdint.h>
#include <string.h>

namespace logiksmith {
namespace openknx {

// A standard KNX APDU carries at most 14 value octets. Keeping this record
// fixed-size is important: the KNX callback must not allocate or block.
constexpr size_t kMaxTelegramPayload = 14;
constexpr size_t kMaxBindings = 32;
constexpr size_t kMaxEndpointName = 31;
constexpr size_t kInputQueueCapacity = 16;
constexpr size_t kOutputQueueCapacity = 16;

struct DptId {
    constexpr DptId(uint16_t main_group = 0, uint16_t subtype_group = 0)
        : main(main_group), subtype(subtype_group) {}

    uint16_t main;
    uint16_t subtype;

    bool operator==(const DptId& other) const {
        return main == other.main && subtype == other.subtype;
    }
};

constexpr DptId kDptBool{1, 1};
constexpr DptId kDptPercent{5, 1};
constexpr DptId kDptTemperature{9, 1};

class EndpointId {
  public:
    EndpointId() = default;

    bool assign(const char* value) {
        if (value == nullptr) {
            clear();
            return false;
        }

        size_t length = 0;
        while (length <= kMaxEndpointName && value[length] != '\0') {
            ++length;
        }
        if (length == 0 || length > kMaxEndpointName) {
            clear();
            return false;
        }

        memcpy(_bytes, value, length);
        _bytes[length] = '\0';
        _length = static_cast<uint8_t>(length);
        return true;
    }

    void clear() {
        _bytes[0] = '\0';
        _length = 0;
    }

    const char* c_str() const { return _bytes; }
    size_t size() const { return _length; }
    bool empty() const { return _length == 0; }

    bool operator==(const EndpointId& other) const {
        return _length == other._length &&
               memcmp(_bytes, other._bytes, _length) == 0;
    }
    bool operator!=(const EndpointId& other) const { return !(*this == other); }

  private:
    char _bytes[kMaxEndpointName + 1] = {};
    uint8_t _length = 0;
};

enum class BindingDirection : uint8_t { Input, Output };

struct Binding {
    uint16_t group_address = 0;
    EndpointId endpoint;
    DptId dpt;
    BindingDirection direction = BindingDirection::Input;
};

struct RawGroupTelegram {
    uint16_t source_address = 0;
    uint16_t destination_address = 0;
    uint8_t payload[kMaxTelegramPayload] = {};
    uint8_t payload_size = 0;
    uint32_t received_at_ms = 0;
};

struct InputEvent {
    EndpointId endpoint;
    DptId dpt;
    uint16_t source_address = 0;
    uint16_t group_address = 0;
    uint8_t payload[kMaxTelegramPayload] = {};
    uint8_t payload_size = 0;
    uint32_t received_at_ms = 0;

    // DPT 1 encodes the value in the low bit of the first value octet.
    bool bool_value() const { return payload_size == 1 && (payload[0] & 0x01U) != 0; }
};

struct OutputEffect {
    EndpointId endpoint;
    DptId dpt;
    uint8_t payload[kMaxTelegramPayload] = {};
    uint8_t payload_size = 0;
};

struct RawGroupValue {
    uint16_t destination_address = 0;
    uint8_t payload[kMaxTelegramPayload] = {};
    uint8_t payload_size = 0;
};

class RawGroupSender {
  public:
    virtual ~RawGroupSender() = default;
    virtual bool send_group_value(uint16_t destination_address,
                                  const uint8_t* payload,
                                  uint8_t payload_size) = 0;
};

template <typename T, size_t Capacity>
class SpscQueue {
    static_assert(Capacity > 0, "queue capacity must be positive");

  public:
    bool push(const T& value) {
        const uint32_t write = _write.load(std::memory_order_relaxed);
        const uint32_t read = _read.load(std::memory_order_acquire);
        if (write - read >= Capacity) {
            return false;
        }
        _slots[write % Capacity] = value;
        _write.store(write + 1, std::memory_order_release);
        return true;
    }

    bool pop(T& value) {
        const uint32_t read = _read.load(std::memory_order_relaxed);
        const uint32_t write = _write.load(std::memory_order_acquire);
        if (read == write) {
            return false;
        }
        value = _slots[read % Capacity];
        _read.store(read + 1, std::memory_order_release);
        return true;
    }

    size_t size() const {
        const uint32_t write = _write.load(std::memory_order_acquire);
        const uint32_t read = _read.load(std::memory_order_acquire);
        return static_cast<size_t>(write - read);
    }

  private:
    T _slots[Capacity] = {};
    std::atomic<uint32_t> _write{0};
    std::atomic<uint32_t> _read{0};
};

enum class BindingTableError : uint8_t {
    None,
    NullBindings,
    TooManyBindings,
    InvalidGroupAddress,
    InvalidEndpoint,
    UnsupportedDpt,
    DuplicateOutput,
    DuplicateGroupAddress,
};

class BindingTable {
  public:
    BindingTableError replace(const Binding* bindings, size_t count);

    size_t size() const { return _size; }
    const Binding* find_input(uint16_t group_address) const;
    const Binding* find_output(const EndpointId& endpoint, DptId dpt) const;

  private:
    static bool valid_dpt(DptId dpt);

    Binding _bindings[kMaxBindings] = {};
    size_t _size = 0;
};

enum class IngressResult : uint8_t {
    Enqueued,
    IgnoredUnbound,
    Malformed,
    QueueFull,
};

enum class OutputResult : uint8_t {
    Enqueued,
    Unbound,
    Malformed,
    QueueFull,
};

struct RouterStats {
    uint32_t telegrams_seen = 0;
    uint32_t telegrams_enqueued = 0;
    uint32_t telegrams_ignored_unbound = 0;
    uint32_t telegrams_malformed = 0;
    uint32_t input_queue_full = 0;
    uint32_t outputs_enqueued = 0;
    uint32_t outputs_unbound = 0;
    uint32_t output_queue_full = 0;
    uint32_t outputs_sent = 0;
    uint32_t outputs_failed = 0;
};

class RawBindingRouter {
  public:
    explicit RawBindingRouter(const BindingTable& bindings) : _bindings(bindings) {}

    IngressResult ingest(const RawGroupTelegram& telegram);
    OutputResult enqueue_output(const EndpointId& endpoint,
                                DptId dpt,
                                const uint8_t* payload,
                                uint8_t payload_size);
    OutputResult enqueue_bool_output(const EndpointId& endpoint, bool value);

    bool pop_input(InputEvent& event) { return _inputs.pop(event); }
    size_t drain_outputs(RawGroupSender& sender, size_t limit = kOutputQueueCapacity);
    RouterStats stats() const;

  private:
    static bool valid_address(uint16_t address) {
        // Zero is the broadcast address, not a user endpoint binding.
        return address != 0 && address <= 0x7FFF;
    }

    const BindingTable& _bindings;
    SpscQueue<InputEvent, kInputQueueCapacity> _inputs;
    SpscQueue<RawGroupValue, kOutputQueueCapacity> _outputs;

    std::atomic<uint32_t> _telegrams_seen{0};
    std::atomic<uint32_t> _telegrams_enqueued{0};
    std::atomic<uint32_t> _telegrams_ignored_unbound{0};
    std::atomic<uint32_t> _telegrams_malformed{0};
    std::atomic<uint32_t> _input_queue_full{0};
    std::atomic<uint32_t> _outputs_enqueued{0};
    std::atomic<uint32_t> _outputs_unbound{0};
    std::atomic<uint32_t> _output_queue_full{0};
    std::atomic<uint32_t> _outputs_sent{0};
    std::atomic<uint32_t> _outputs_failed{0};
};

} // namespace openknx
} // namespace logiksmith
