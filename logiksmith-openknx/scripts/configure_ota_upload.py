Import("env")

import ast
import re
import shlex
from pathlib import Path


if env["PIOENV"].endswith("_ota"):
    project_dir = Path(env.subst("$PROJECT_DIR"))
    wifi_header = project_dir.parent / "config" / "wifi.h"
    if not wifi_header.is_file():
        raise RuntimeError("Copy config/wifi.h.example to config/wifi.h before OTA upload")

    source = wifi_header.read_text()
    match = re.search(
        r'^\s*#\s*define\s+LOGIKSMITH_OTA_PASSWORD\s+("(?:\\.|[^"\\])*")\s*$',
        source,
        re.MULTILINE,
    )
    if match is None:
        raise RuntimeError("config/wifi.h must define LOGIKSMITH_OTA_PASSWORD as a string")

    password = ast.literal_eval(match.group(1))
    if not password:
        raise RuntimeError("Set LOGIKSMITH_OTA_PASSWORD in config/wifi.h before OTA upload")

    # PlatformIO expands UPLOAD_FLAGS into the ESP32 platform's
    # UPLOADERFLAGS after its builder script has selected the espota tool.
    # Keep the password as one shell token, including when it has spaces.
    env.Replace(UPLOAD_FLAGS=shlex.quote("--auth=" + password))
