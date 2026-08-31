"""Apply the narrow raw-telegram extension to the pinned OpenKNX checkout.

PlatformIO installs git dependencies in a generated libdeps directory. The
extension intentionally remains a small, reviewable patch in this repository;
it is not a fork or a copy of the OpenKNX stack.
"""

from pathlib import Path
import subprocess

Import("env")

configured_project_dir = Path(env.subst("$PROJECT_DIR"))
project_config = Path(env.subst("$PROJECT_CONFIG"))
project_candidates = []
for candidate in (configured_project_dir, project_config.parent):
    if not candidate.is_absolute():
        candidate = Path.cwd() / candidate
    candidate = candidate.resolve()
    if candidate not in project_candidates:
        project_candidates.append(candidate)
# SCons normally supplies an absolute PROJECT_CONFIG, but the command
# `pio run --project-dir ...` has also exposed a workspace-root PROJECT_DIR on
# older PlatformIO releases. The script location is the final deterministic
# fallback when both values are relative or otherwise misleading.
script_file = globals().get("__file__")
if script_file:
    script_dir = Path(script_file).resolve().parent.parent
    if script_dir not in project_candidates:
        project_candidates.append(script_dir)

patch_project_dir = next(
    (
        candidate
        for candidate in project_candidates
        if (candidate / "patches" / "knx-raw-group-hooks.patch").is_file()
    ),
    None,
)
if patch_project_dir is None:
    raise RuntimeError(
        "LogikSmith raw OpenKNX patch is missing; checked: "
        + ", ".join(str(candidate / "patches") for candidate in project_candidates)
    )
patch_path = patch_project_dir / "patches" / "knx-raw-group-hooks.patch"
libdeps_roots = []
for project_dir in project_candidates:
    libdeps_root = project_dir / ".pio" / "libdeps"
    if libdeps_root not in libdeps_roots:
        libdeps_roots.append(libdeps_root)

candidates = []
for libdeps_root in libdeps_roots:
    candidates.extend(
        path
        for path in libdeps_root.glob("*/knx")
        if (path / "src" / "knx" / "transport_layer.h").is_file()
    )
if len(candidates) != 1:
    raise RuntimeError(
        "expected one pinned OpenKNX knx libdep, found: "
        + ", ".join(str(path) for path in candidates)
    )

knx_root = candidates[0]
hardware_candidates = []
for libdeps_root in libdeps_roots:
    for hardware_root in libdeps_root.glob("*/OGM-HardwareConfig"):
        hardware_include = hardware_root / "include"
        if (hardware_include / "HardwareConfig.h").is_file():
            hardware_candidates.append(hardware_include)
if len(hardware_candidates) != 1:
    raise RuntimeError(
        "expected one OGM-HardwareConfig include directory, found: "
        + ", ".join(str(path) for path in hardware_candidates)
    )
# CPPPATH is sufficient for project sources, but PlatformIO creates a
# separate environment for library sources. BUILD_FLAGS is inherited by both,
# which keeps OGM-Common's hardware.h include resolvable too.
hardware_flag = "-I" + str(hardware_candidates[0])
env.Append(CPPPATH=[str(hardware_candidates[0])], BUILD_FLAGS=[hardware_flag])

transport_header = knx_root / "src" / "knx" / "transport_layer.h"
transport_source = knx_root / "src" / "knx" / "transport_layer.cpp"
bau_header = knx_root / "src" / "knx" / "bau_systemB_device.h"
bau_source = knx_root / "src" / "knx" / "bau_systemB_device.cpp"
markers = {
    transport_header: "RawGroupTelegramObserver",
    transport_source: "dataGroupValueRequestRaw",
    bau_header: "rawGroupValueWrite",
    bau_source: "rawGroupValueWrite",
}
patched = [marker in path.read_text() for path, marker in markers.items()]
if not any(patched):
    check = subprocess.run(
        ["git", "apply", "--check", "--recount", str(patch_path)],
        cwd=knx_root,
        capture_output=True,
        text=True,
    )
    if check.returncode != 0:
        raise RuntimeError("OpenKNX raw patch does not apply:\n" + check.stderr)

    subprocess.run(
        ["git", "apply", "--whitespace=nowarn", "--recount", str(patch_path)],
        cwd=knx_root,
        check=True,
    )
elif not all(patched):
    raise RuntimeError("OpenKNX raw patch is only partially applied")
