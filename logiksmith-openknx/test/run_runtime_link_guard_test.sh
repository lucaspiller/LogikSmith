#!/usr/bin/env sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
elf=${1:-"$root/.pio/build/release_LogikSmith_REG1_LAN_TP_Base/firmware.elf"}
nm=${NM:-xtensa-esp32-elf-nm}

if [ ! -f "$elf" ]; then
    echo "runtime link guard: firmware ELF does not exist: $elf" >&2
    exit 2
fi
if ! command -v "$nm" >/dev/null 2>&1 && [ -x "$HOME/.platformio/packages/toolchain-xtensa-esp-elf/bin/xtensa-esp32-elf-nm" ]; then
    nm="$HOME/.platformio/packages/toolchain-xtensa-esp-elf/bin/xtensa-esp32-elf-nm"
fi
if ! command -v "$nm" >/dev/null 2>&1 && [ ! -x "$nm" ]; then
    echo "runtime link guard: nm tool is unavailable: $nm" >&2
    exit 2
fi

symbols=$("$nm" --defined-only "$elf")
missing=0
for symbol in \
    logiksmith_abi_version \
    logiksmith_runtime_create \
    logiksmith_runtime_destroy \
    logiksmith_runtime_process_input; do
    if ! printf '%s\n' "$symbols" | grep -Eq "[[:space:]]${symbol}(\$|[[:space:]])"; then
        echo "runtime link guard: missing $symbol" >&2
        missing=1
    fi
done

if [ "$missing" -ne 0 ]; then
    echo "runtime link guard: refusing an image that would select DisabledRuntimeProcessor" >&2
    exit 1
fi
echo "runtime link guard: embedded ABI symbols are linked"
