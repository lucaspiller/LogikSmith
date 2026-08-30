# LogikSmith OpenKNX host

This directory is the M14 firmware host for the OpenKNX `REG1-LAN-TP-Base`
profile (classic ESP32, 8 MB flash). It is an OpenKNX module, so it can be
registered beside another application module in the same firmware image.

`RawBindingRouter` is the platform-independent part of the host boundary. It
copies a raw group telegram into a fixed-size SPSC queue only after matching
the destination address in the local binding table. Output effects resolve the
logical endpoint back to the current configured group address and use a second
bounded queue. The KNX callback never allocates, waits, or calls Rust.

The OpenKNX stack's normal GroupObject route remains untouched for other
modules. The small patch in `patches/knx-raw-group-hooks.patch` adds the raw
observer and direct group-value sender used by this module; the PlatformIO
extra script applies it to the pinned libdep checkout at build time.

M14 includes a checked-in default binding scaffold in `data/` and equivalent
compiled defaults. M15 can replace the table from its web/config store without
creating ETS associations for LogikSmith endpoints.

## Host seam

`RuntimeProcessor` is the explicit boundary between the OpenKNX loop and the
portable Rust ABI. M14 includes a weak-linked `AbiRuntimeProcessor` for the
default boolean block and selects it when the ABI symbols are present. The
firmware otherwise selects an explicit `DisabledRuntimeProcessor` because the
current core still depends on desktop `std`/vendored Lua and cannot yet be
linked for classic ESP32 Xtensa. The raw KNX callback remains transport-only.
Native tests exercise both the router and ABI processor without requiring
OpenKNX or a device.

## Checks

```sh
./test/run_raw_binding_router_test.sh
./test/run_abi_runtime_processor_test.sh
```

The full PlatformIO build requires PlatformIO, the Espressif Xtensa toolchain,
and network access for the pinned OpenKNX dependencies. It builds this
directory only; the external demo project is reference material and is never a
build input.
