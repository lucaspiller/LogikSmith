#!/bin/sh
set -eu

REPO_ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$REPO_ROOT"

if ! command -v mise >/dev/null 2>&1; then
    echo "error: mise is required; install it and rerun ./scripts/bootstrap.sh" >&2
    exit 1
fi

echo "Installing mise-managed toolchains..."
mise install

if [ ! -x .venv/bin/python ]; then
    echo "Creating Python virtual environment..."
    mise exec -- python -m venv .venv
fi

BRIDGE_DIR=${BRIDGE_DIR:-bridges/xknx}
BRIDGE_REQUIREMENTS="$BRIDGE_DIR/requirements.txt"

if [ ! -f "$BRIDGE_REQUIREMENTS" ]; then
    echo "error: bridge dependency lock not found: $BRIDGE_REQUIREMENTS" >&2
    exit 1
fi
if [ ! -f "$BRIDGE_DIR/pyproject.toml" ] && [ ! -f "$BRIDGE_DIR/setup.py" ]; then
    echo "error: local bridge package metadata not found in $BRIDGE_DIR" >&2
    exit 1
fi

echo "Installing the local bridge and its locked dependency..."
.venv/bin/python -m pip install --requirement "$BRIDGE_REQUIREMENTS"
.venv/bin/python -m pip install --no-deps --editable "$BRIDGE_DIR"

echo "Building the Rust workspace..."
mise exec -- cargo build --workspace

echo "Bootstrap complete. Next steps:"
echo "  cp config/local.toml.example config/local.toml"
echo "  edit config/local.toml with a verified, safe KNX input/output"
echo "  ./scripts/run-dev.sh"
