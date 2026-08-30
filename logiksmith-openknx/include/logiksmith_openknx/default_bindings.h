#pragma once

#include "logiksmith_openknx/raw_binding_router.h"

namespace logiksmith {
namespace openknx {

// Three-level KNX group addresses: main (5 bits), middle (3 bits), subgroup
// (8 bits). The scaffold therefore matches 1/1/1 and 1/1/2 from data/.
constexpr uint16_t kDefaultTriggerGroupAddress = (1U << 11) | (1U << 8) | 1U;
constexpr uint16_t kDefaultLightGroupAddress = (1U << 11) | (1U << 8) | 2U;

// M14 deliberately ships a tiny deterministic configuration. The web editor
// can replace this table in M15 without changing ETS group-object tables.
inline Binding default_binding(const char* endpoint,
                               uint16_t group_address,
                               DptId dpt,
                               BindingDirection direction) {
    Binding binding;
    binding.group_address = group_address;
    binding.endpoint.assign(endpoint);
    binding.dpt = dpt;
    binding.direction = direction;
    return binding;
}

inline BindingTableError load_default_bindings(BindingTable& table) {
    Binding bindings[2];
    bindings[0] = default_binding("trigger", kDefaultTriggerGroupAddress, kDptBool,
                                  BindingDirection::Input);
    bindings[1] = default_binding("light", kDefaultLightGroupAddress, kDptBool,
                                  BindingDirection::Output);
    return table.replace(bindings, 2);
}

} // namespace openknx
} // namespace logiksmith
