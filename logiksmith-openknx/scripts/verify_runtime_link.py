"""Fail a release image that would fall back to DisabledRuntimeProcessor.

The C++ host uses weak declarations so native/OpenKNX-only builds can still be
compiled while the Xtensa Rust runtime is being ported. A release firmware
must not be accepted under that fallback, however. This post-link check makes
the missing-runtime state a build failure instead of a silent no-op device.
"""

from pathlib import Path
import shutil
import subprocess


Import("env")


REQUIRED_SYMBOLS = (
    "logiksmith_abi_version",
    "logiksmith_runtime_create",
    "logiksmith_runtime_destroy",
    "logiksmith_runtime_process_input",
)


def _nm_command() -> str | None:
    configured = env.subst("$NM")
    if configured and configured != "$NM":
        return configured

    compiler = env.subst("$CC")
    if compiler and compiler != "$CC":
        candidate = Path(compiler).with_name(Path(compiler).name.replace("-gcc", "-nm"))
        if candidate.is_file():
            return str(candidate)
    return shutil.which("nm")


def verify_runtime_link(source, target, env) -> None:
    del source, env
    elf = Path(str(target[0]))
    nm = _nm_command()
    if nm is None:
        raise RuntimeError("cannot verify LogikSmith runtime: Xtensa nm tool is unavailable")

    result = subprocess.run(
        [nm, "--defined-only", str(elf)],
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise RuntimeError("cannot inspect release image symbols:\n" + result.stderr)

    symbols = result.stdout
    missing = [symbol for symbol in REQUIRED_SYMBOLS if symbol not in symbols]
    if missing:
        names = ", ".join(missing)
        raise RuntimeError(
            "release image is missing the LogikSmith embedded ABI symbols: "
            + names
            + "; refusing an image that would select DisabledRuntimeProcessor"
        )

    print("LogikSmith release runtime guard: embedded ABI symbols are linked")


env.AddPostAction("$BUILD_DIR/${PROGNAME}.elf", verify_runtime_link)
