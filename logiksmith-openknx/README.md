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
portable Rust ABI. M14 includes an `AbiRuntimeProcessor` for the default
boolean block; native tests use weak declarations so they can run without the
archive. The release environment requires the embedded ABI
symbols at post-link time and fails closed at startup if they are unavailable;
it cannot silently run `DisabledRuntimeProcessor`. The raw KNX callback remains
transport-only. Native tests exercise both the router and ABI processor without
requiring OpenKNX or a device.

## Checks

```sh
./test/run_raw_binding_router_test.sh
./test/run_abi_runtime_processor_test.sh
./test/run_runtime_link_guard_test.sh
```

The full PlatformIO build requires PlatformIO, the Espressif Xtensa toolchain,
and network access for the pinned OpenKNX dependencies. Its pre-build Rust
step compiles `../crates/logiksmith-embedded-abi` for
`xtensa-esp32-espidf` with target `std` and Lua 5.4, checks the exported ABI
symbols, and links that archive into the image. Release C++ uses strong ABI
references and a post-link guard, so missing runtime code is a build failure
rather than a disabled processor image. It builds this directory and the
repository crate only; the external demo project is reference material and is
never a build input.
