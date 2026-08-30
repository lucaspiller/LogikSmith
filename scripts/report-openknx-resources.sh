#!/bin/sh
# Report measured artifacts for the classic ESP32 OpenKNX profile.
#
# This command is intentionally report-only unless --build is supplied. It
# never uses the historical OAM-LogicMachine demo directory. Values printed in
# the "provisional" section are planning limits, not measurements.
set -eu

REPO_ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
PROJECT_DIR="$REPO_ROOT/logiksmith-openknx"
ENVIRONMENT="release_LogikSmith_REG1_LAN_TP_Base"
BUILD=0
SIZE=0

usage() {
    cat <<EOF
Usage: $0 [options]

Options:
  --project-dir PATH  OpenKNX app directory (default: $PROJECT_DIR)
  --environment NAME  PlatformIO environment (default: $ENVIRONMENT)
  --build             Build the selected environment before reporting
  --size              Ask PlatformIO for its RAM/flash size summary
  -h, --help          Show this help
EOF
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --project-dir)
            [ "$#" -ge 2 ] || { echo "error: --project-dir needs a path" >&2; exit 2; }
            PROJECT_DIR=$2
            shift 2
            ;;
        --environment)
            [ "$#" -ge 2 ] || { echo "error: --environment needs a name" >&2; exit 2; }
            ENVIRONMENT=$2
            shift 2
            ;;
        --build)
            BUILD=1
            shift
            ;;
        --size)
            SIZE=1
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "error: unknown option: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

echo "OpenKNX resource report (classic ESP32 / REG1-LAN-TP-Base)"
echo "Project: $PROJECT_DIR"
echo "Environment: $ENVIRONMENT"
echo
echo "Measured values"

if [ ! -d "$PROJECT_DIR" ]; then
    echo "  firmware artifact: unavailable (project directory does not exist)"
    echo "  PlatformIO size summary: unavailable (project directory does not exist)"
else
    if [ "$BUILD" -eq 1 ]; then
        if ! command -v pio >/dev/null 2>&1; then
            echo "error: --build requested but PlatformIO (pio) is not installed" >&2
            exit 1
        fi
        pio run --project-dir "$PROJECT_DIR" --environment "$ENVIRONMENT"
    fi

    BUILD_DIR="$PROJECT_DIR/.pio/build/$ENVIRONMENT"
    artifact_bytes=0
    artifact_name=""
    if [ -d "$BUILD_DIR" ]; then
        for artifact in "$BUILD_DIR"/*.bin; do
            [ -f "$artifact" ] || continue
            # `wc -c` is portable across macOS and Linux (unlike the two
            # incompatible `stat` format syntaxes).
            bytes=$(wc -c <"$artifact" | tr -d ' ')
            if [ "$bytes" -gt "$artifact_bytes" ]; then
                artifact_bytes=$bytes
                artifact_name=$artifact
            fi
        done
    fi
    if [ -n "$artifact_name" ]; then
        echo "  firmware artifact: $artifact_name ($artifact_bytes bytes)"
    else
        echo "  firmware artifact: unavailable (run with --build)"
    fi

    if [ "$SIZE" -eq 1 ]; then
        if ! command -v pio >/dev/null 2>&1; then
            echo "  PlatformIO size summary: unavailable (pio is not installed)"
        else
            size_log=$(mktemp "${TMPDIR:-/tmp}/logiksmith-openknx-size.XXXXXX")
            trap 'rm -f "$size_log"' EXIT
            # The pinned pioarduino platform ships an esp-idf-size wrapper
            # that rejects PlatformIO's newer `--ng` option when invoked via
            # `-t size`. A normal incremental build still emits the canonical
            # RAM/Flash summary, so use that output as the portable source.
            if pio run --project-dir "$PROJECT_DIR" --environment "$ENVIRONMENT" >"$size_log" 2>&1; then
                awk '/RAM:|Flash:|Memory Usage/ { print "  " $0 }' "$size_log"
                if ! grep -Eq 'RAM:|Flash:|Memory Usage' "$size_log"; then
                    echo "  PlatformIO size summary: command succeeded but emitted no parsable summary"
                fi
            else
                echo "  PlatformIO size summary: unavailable (pio size command failed)"
                sed -n '1,12p' "$size_log" >&2
            fi
        fi
    else
        echo "  PlatformIO size summary: not requested (rerun with --size)"
    fi
fi

echo
echo "Provisional planning budgets (not measurements)"
echo "  internal RAM: 320 KiB reported by PlatformIO for target hardware"
echo "  external flash: 8 MiB reported for target hardware"
echo "  app headroom: reserve at least 10% after the measured firmware artifact"
echo "  runtime queue: bounded; exact count is an implementation decision"
