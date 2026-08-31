"""Build and link the real LogikSmith ABI for the OpenKNX release image.

PlatformIO drives the final Xtensa link, while Cargo builds the portable ABI
crate as a target-compatible static archive. This script intentionally points
at this repository's crate; the historical OAM-LogicMachine project is never
used as a build input.
"""

from pathlib import Path
import os
import shutil
import subprocess


Import("env")


def _project_directory() -> Path:
    configured = Path(env.subst("$PROJECT_DIR"))
    config = Path(env.subst("$PROJECT_CONFIG"))
    candidates = [configured]
    if config.is_file():
        candidates.append(config.parent)
    for candidate in candidates:
        candidate = candidate.resolve()
        if (candidate / "platformio.ini").is_file():
            return candidate
    raise RuntimeError(
        "cannot locate the logiksmith-openknx project from PlatformIO "
        f"(PROJECT_DIR={configured}, PROJECT_CONFIG={config})"
    )


project_dir = _project_directory()
repository_dir = project_dir.parent
manifest = repository_dir / "crates" / "logiksmith-embedded-abi" / "Cargo.toml"
if not manifest.is_file():
    raise RuntimeError("embedded ABI Cargo manifest is missing: " + str(manifest))

toolchain_name = os.environ.get("LOGIKSMITH_RUST_TOOLCHAIN", "esp")
target = os.environ.get("LOGIKSMITH_RUST_TARGET", "xtensa-esp32-espidf")
toolchain_dir = Path.home() / ".rustup" / "toolchains" / toolchain_name
rust_bin = toolchain_dir / "bin"
pio_bin = Path.home() / ".platformio" / "packages" / "toolchain-xtensa-esp-elf" / "bin"
if not rust_bin.is_dir() or not pio_bin.is_dir():
    raise RuntimeError(
        "ESP Rust/PlatformIO toolchains are unavailable; expected "
        f"{toolchain_dir} and {pio_bin}"
    )
gcc_bin = pio_bin
build_path = os.pathsep.join(
    [str(rust_bin), str(gcc_bin), os.environ.get("PATH", "")]
)
build_env = os.environ.copy()
build_env["PATH"] = build_path
build_env["RUSTUP_TOOLCHAIN"] = toolchain_name
# The Rust toolchain's generic xtensa-esp-elf compiler defaults to big-endian,
# while both the Rust target and PlatformIO firmware are little-endian ESP32.
# Use PlatformIO's exact ESP32 compiler for the vendored Lua C dependency.
compiler = gcc_bin / "xtensa-esp32-elf-gcc"
archiver = gcc_bin / "xtensa-esp32-elf-ar"
nm_tool = gcc_bin / "xtensa-esp32-elf-nm"
if not compiler.is_file() or not archiver.is_file():
    raise RuntimeError(f"ESP32 Xtensa compiler tools are missing from {gcc_bin}")
build_env["LOGIKSMITH_XTENSA_GCC"] = str(compiler)
build_env["LOGIKSMITH_XTENSA_AR"] = str(archiver)
target_env_suffix = target.replace("-", "_")
build_env.setdefault(f"CC_{target_env_suffix}", str(compiler))
build_env.setdefault(f"AR_{target_env_suffix}", str(archiver))

target_dir = Path(build_env.get("CARGO_TARGET_DIR", repository_dir / "target"))
if not target_dir.is_absolute():
    target_dir = repository_dir / target_dir
build_env["LUA_LIB_NAME"] = "lua54"
build_env["LUA_LINK"] = "static"
cargo = shutil.which("cargo", path=build_path)
if cargo is None:
    raise RuntimeError("cargo is unavailable in the ESP Rust toolchain")

command = [
    cargo,
    "-Zbuild-std=std,panic_abort",
    "build",
    "--manifest-path",
    str(manifest),
    "--release",
    "--no-default-features",
    "--target",
    target,
]
rustflags = build_env.get("RUSTFLAGS", "").strip()
if "-Zunstable-options" not in rustflags:
    rustflags = f"{rustflags} -Zunstable-options".strip()
if "panic=immediate-abort" not in rustflags:
    rustflags = f"{rustflags} -C panic=immediate-abort".strip()
build_env["RUSTFLAGS"] = rustflags
features = os.environ.get("LOGIKSMITH_RUST_FEATURES", "embedded-lua").strip()
if features:
    command.extend(["--features", features])
subprocess.run(command, cwd=repository_dir, env=build_env, check=True)

archive_dir = target_dir / target / "release"
archive = archive_dir / "liblogiksmith_embedded_abi.a"
if not archive.is_file():
    raise RuntimeError("Cargo did not produce the embedded ABI archive: " + str(archive))

nm = nm_tool
if not nm.is_file():
    nm = Path(shutil.which("xtensa-esp32-elf-nm", path=build_path) or "")
if not nm.is_file():
    raise RuntimeError("xtensa nm tool is unavailable; cannot verify the ABI archive")
symbols = subprocess.run(
    [str(nm), "--defined-only", str(archive)],
    check=True,
    capture_output=True,
    text=True,
).stdout
required_symbols = (
    "logiksmith_abi_version",
    "logiksmith_runtime_create",
    "logiksmith_runtime_destroy",
    "logiksmith_runtime_process_input",
)
missing = [symbol for symbol in required_symbols if symbol not in symbols]
if missing:
    raise RuntimeError(
        "embedded ABI archive is missing required symbols: " + ", ".join(missing)
    )

# LIBS is deliberately paired with strong ABI declarations in release C++.
# Thus the archive is extracted by the linker instead of being discarded as a
# library referenced only through weak symbols in native test builds.
env.Append(
    LIBPATH=[str(archive_dir)],
    LIBS=["logiksmith_embedded_abi"],
)
print("LogikSmith Rust ABI: linked " + str(archive))
print("LogikSmith target Lua: compiled by the embedded mlua source patch")
