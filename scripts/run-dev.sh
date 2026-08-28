#!/bin/sh
set -eu

REPO_ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$REPO_ROOT"

if ! command -v mise >/dev/null 2>&1; then
    echo "error: mise is required; run ./scripts/bootstrap.sh first" >&2
    exit 1
fi
if [ ! -f config/local.toml ]; then
    echo "error: config/local.toml is missing; copy config/local.toml.example and edit it first" >&2
    exit 1
fi
if [ ! -f config/automation.toml ]; then
    echo "error: config/automation.toml is missing; copy config/automation.toml.example and edit it first" >&2
    exit 1
fi
if [ ! -x .venv/bin/python ]; then
    echo "error: .venv/bin/python is missing; run ./scripts/bootstrap.sh first" >&2
    exit 1
fi

if [ ! -d logiksmith-web/node_modules ]; then
    mise exec -- npm --prefix logiksmith-web ci
fi
mise exec -- npm --prefix logiksmith-web run build

exec mise exec -- cargo run -p logiksmith-desktop -- --config config/local.toml --automation config/automation.toml
