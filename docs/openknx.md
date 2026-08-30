# Building and configuring the OpenKNX host

`logiksmith-openknx/` is the M14 firmware host for the OpenKNX
`REG1-LAN-TP-Base` profile. It keeps the OpenKNX device shell and KNX TP
transport, then adds a LogikSmith module beside the other OpenKNX modules.
The Rust core is reached through a small C ABI when a compatible embedded
runtime is linked.

The target profile is a classic ESP32 at 240 MHz with 320 KiB reported RAM,
8 MiB flash, NanoBCU KNX TP at 38.4 kbaud, and a 10/100 RMII Ethernet PHY.
The PlatformIO environment uses the `esp32dev` board definition with the
target-specific OpenKNX and partition settings from `platformio.ini`.

## Prerequisites

Install PlatformIO and make sure its `pio` command is on `PATH`. The first
build also needs network access because the project pins the OpenKNX, OGM
Common, OGM HardwareConfig, and Espressif platform dependencies by revision.

The build uses this repository only. The older
`/Users/luca/code/personal/openknx/OAM-LogicMachine/` project is a reference
for the toolchain and is not a dependency or build input.

## Build

From the repository root, build the release image with:

```sh
pio run \
  --project-dir logiksmith-openknx \
  --environment release_LogikSmith_REG1_LAN_TP_Base
```

The equivalent command from the application directory is:

```sh
cd logiksmith-openknx
pio run -e release_LogikSmith_REG1_LAN_TP_Base
```

PlatformIO places generated files in `logiksmith-openknx/.pio/`. That
directory is ignored by Git. The build script applies
`patches/knx-raw-group-hooks.patch` to the generated pinned KNX checkout. The
patch adds a raw group-telegram observer and a direct group-value sender; it
does not fork or copy the OpenKNX stack into this repository.

To build and print the measured firmware, RAM, and flash figures:

```sh
./scripts/report-openknx-resources.sh --build --size
```

The pinned PlatformIO platform can print a harmless `esp-idf-size --ng`
warning during this command. The normal RAM/Flash summary and a successful
PlatformIO result are the useful signals.

## Upload and monitor

Connect the target board, check the port PlatformIO can see, then upload the
same environment:

```sh
pio device list
pio run \
  --project-dir logiksmith-openknx \
  --environment release_LogikSmith_REG1_LAN_TP_Base \
  --target upload
pio device monitor \
  --project-dir logiksmith-openknx \
  --environment release_LogikSmith_REG1_LAN_TP_Base
```

The monitor speed is 115200 baud. If more than one serial port is present,
select the board explicitly with PlatformIO's `upload_port` and
`monitor_port` options or the corresponding command-line options.

M14 has not been verified on a physical REG1-LAN-TP-Base device. Treat upload
as a preparation step until a device test is available.

## Default group-address bindings

M14 ships a small deterministic binding table so the host can be exercised
without an ETS application configuration for LogikSmith endpoints:

```text
input trigger 1/1/1 dpt1.001
output light 1/1/2 dpt1.001
```

The text file at
`logiksmith-openknx/data/logiksmith-bindings.conf` documents that shape. It
is not loaded by the M14 firmware yet. The authoritative M14 defaults are in
`logiksmith-openknx/include/logiksmith_openknx/default_bindings.h`.

To change the defaults for a build, edit that header and update the two
`Binding` entries passed to `BindingTable::replace`, then rebuild. A three-
level KNX group address is encoded as:

```text
(main << 11) | (middle << 8) | subgroup
```

For example, `1/1/1` is `0x0901` and `1/1/2` is `0x0902`. Addresses must be
nonzero 15-bit group addresses. The router currently validates DPT 1.001
(boolean), DPT 5.001 (percentage), and DPT 9.001 (2-byte float temperature)
payload shapes. The optional Rust ABI adapter in M14 currently exercises the
boolean path.

The future web/config store can replace the table at runtime through the
same `BindingTable::replace` seam. That is the M15 configuration work; editing
`data/logiksmith-bindings.conf` alone does not change a flashed image.

## ETS and raw group traffic

The LogikSmith module listens to raw `GroupValueWrite` telegrams before the
OpenKNX association-table lookup. The local binding table then decides which
destinations become LogikSmith input events. This is why the two LogikSmith
endpoint addresses do not need ETS group-object associations.

The normal OpenKNX GroupObject path remains available to other modules in the
same image. ETS commissioning can still be needed for the base device or for
those other modules; it is not used as the source of truth for the M14
LogikSmith binding table.

Output effects use the current local binding table to resolve an endpoint back
to a group address and send a raw group value. Input and output addresses are
kept separate in the default configuration so a test cannot accidentally
create a feedback loop.

## Runtime selection

At startup, `LogicSmithModule` tries to create `AbiRuntimeProcessor`. It does
so only when the linked image provides the matching
`logiksmith-embedded-abi` symbols and ABI version. When those symbols are not
present, the module selects `DisabledRuntimeProcessor` and the raw transport
and binding-router tests still remain usable.

The current portable core depends on desktop `std`, vendored Lua 5.4, and
`jiff`, so it is not yet linkable as a complete classic ESP32 Xtensa runtime.
As a result, a successful PlatformIO build proves the OpenKNX host, raw hook,
and optional ABI seam compile and link. It does not prove Lua execution on the
device.

## Checks before hardware work

Run the native host checks and the Rust contract tests from the repository
root:

```sh
./logiksmith-openknx/test/run_raw_binding_router_test.sh
./logiksmith-openknx/test/run_abi_runtime_processor_test.sh
cargo test -p logiksmith-embedded-abi
cargo test -p logiksmith-core --test openknx_host_contract
```

Before a manual KNX test, use a harmless visible actuator, keep input and
output group addresses different, and use plain KNXnet/IP tunnelling if a
gateway is part of the test setup. For a fully linked runtime with the timer
logic enabled, the acceptance sequence is a connected device, an input
`true`, an immediate output `true`, and an output `false` after the configured
delay. Retriggering and shutdown should be tested separately. The current M14
OpenKNX image has no timer behavior until a portable Rust runtime is linked.

## Troubleshooting

### The OpenKNX patch fails to apply

The extra script expects exactly one generated `knx` libdep and checks all
four patch markers. Remove stale PlatformIO build products with:

```sh
pio run --project-dir logiksmith-openknx \
  --environment release_LogikSmith_REG1_LAN_TP_Base \
  --target clean
```

Then run the build again. A partial patch is reported as an error so an
incomplete generated checkout cannot be used silently.

### The ABI processor is not started

This is expected when the Rust ABI static library is not linked or its ABI
version does not match. The module falls back to the disabled processor. Use
the native ABI test to validate the adapter independently, and track full
Xtensa portability as a follow-up milestone.

### Dependency downloads fail

Retry with network access and leave the pinned revisions unchanged. Do not
point `platformio.ini` at the external demo project or at an unpinned OpenKNX
checkout, since that makes the patch and resource measurements non-
reproducible.
