#!/usr/bin/env sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cxx=${CXX:-c++}
build_dir=${TMPDIR:-/tmp}/logiksmith-openknx-host-test
mkdir -p "$build_dir"

"$cxx" -std=c++11 -Wall -Wextra -Werror \
    -I"$root/include" \
    -I"$root/../crates/logiksmith-embedded-abi/include" \
    "$root/src/abi_runtime_processor.cpp" \
    "$root/src/raw_binding_router.cpp" \
    "$root/test/abi_runtime_processor_test.cpp" \
    -o "$build_dir/abi_runtime_processor_test"
"$build_dir/abi_runtime_processor_test"
