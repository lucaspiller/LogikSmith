#!/bin/sh
set -eu

REPO_ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$REPO_ROOT"

if ! command -v mise >/dev/null 2>&1; then
    echo "error: mise is required; run ./scripts/bootstrap.sh first" >&2
    exit 1
fi

echo "Building production dashboard..."
mise exec -- npm --prefix logiksmith-web ci
mise exec -- npm --prefix logiksmith-web run build

echo "Building optimized desktop binary..."
mise exec -- cargo build --release -p logiksmith-desktop

echo "Production release ready:"
echo "  binary:  $REPO_ROOT/target/release/logiksmith-desktop"
echo "  assets:  $REPO_ROOT/logiksmith-web/dist"
