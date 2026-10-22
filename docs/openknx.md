# OpenKNX host

The OpenKNX host runs LogikSmith on the `REG1-LAN-TP-Base` ESP32 profile. The
release image includes the embedded Rust runtime and uses a two-slot OTA
partition layout.

## Build

Install PlatformIO, espup's ESP Rust toolchain, and PlatformIO's ESP32 compiler.
From the repository root, build the release image with:

```sh
pio run --project-dir logiksmith-openknx \
  --environment release_LogikSmith_REG1_LAN_TP_Base
```

The build fetches pinned OpenKNX dependencies and stops if the embedded Rust
runtime cannot be linked.

## USB upload

Connect the board by USB and upload the release image:

```sh
pio run --project-dir logiksmith-openknx \
  --environment release_LogikSmith_REG1_LAN_TP_Base \
  --target upload
```

To view startup messages, run:

```sh
pio device monitor --project-dir logiksmith-openknx \
  --environment release_LogikSmith_REG1_LAN_TP_Base
```

The serial monitor uses 115200 baud. The USB release environment is the
recovery path if Wi-Fi or OTA is unavailable.

## Wi-Fi and OTA

Copy `config/wifi.h.example` to `config/wifi.h` in the repository root and set
the Wi-Fi name, Wi-Fi password, and OTA password. The local file is ignored by
Git. The `REG1-LAN-TP-Base` board needs a usable ESP32 antenna for Wi-Fi.

After boot, read the device IP from serial and upload over Wi-Fi:

```sh
pio run --project-dir logiksmith-openknx \
  --environment release_LogikSmith_REG1_LAN_TP_Base_ota \
  --target upload \
  --upload-port 192.168.1.42
```

When Wi-Fi and OTA are configured, runtime startup waits for OTA readiness or
15 seconds, whichever comes first. Runtime initialization runs in a separate
task, while the OpenKNX loop services OTA. After a panic or watchdog reset,
the device skips runtime startup and keeps OTA available for recovery. The
OpenKNX watchdog is enabled with a 60-second maximum period.

## KNX behavior

The default binding maps input `1/1/1` to output `1/1/2`. The LogikSmith
module reads raw group writes and sends outputs as raw group values; the
defaults are in `logiksmith-openknx/include/logiksmith_openknx/default_bindings.h`.

Boot, Wi-Fi, and OTA have been exercised on the device. KNX telegram behavior
still needs a bus-connected test. For that test, use a harmless visible
actuator, keep input and output addresses different, and use plain
KNXnet/IP tunnelling if using a gateway.
