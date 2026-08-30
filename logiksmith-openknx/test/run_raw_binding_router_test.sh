#!/usr/bin/env sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cxx=${CXX:-c++}
build_dir=${TMPDIR:-/tmp}/logiksmith-openknx-host-test
mkdir -p "$build_dir"

"$cxx" -std=c++11 -Wall -Wextra -Werror \
    -I"$root/include" \
    "$root/src/raw_binding_router.cpp" \
    "$root/test/raw_binding_router_test.cpp" \
    -o "$build_dir/raw_binding_router_test"
"$build_dir/raw_binding_router_test"
