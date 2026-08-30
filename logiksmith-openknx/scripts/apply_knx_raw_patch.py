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
script_project_dir = project_config.parent
project_dir = (
    configured_project_dir
    if (configured_project_dir / "patches" / "knx-raw-group-hooks.patch").is_file()
    else script_project_dir
)
patch_path = script_project_dir / "patches" / "knx-raw-group-hooks.patch"
libdeps_roots = [project_dir / ".pio" / "libdeps"]
if script_project_dir / ".pio" / "libdeps" not in libdeps_roots:
    libdeps_roots.append(script_project_dir / ".pio" / "libdeps")

if not patch_path.is_file():
    raise RuntimeError("LogikSmith raw OpenKNX patch is missing")

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
if len(hardware_candidates) == 1:
    env.Append(CPPPATH=[str(hardware_candidates[0])])

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
