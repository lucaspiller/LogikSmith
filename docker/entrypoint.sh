#!/bin/sh
set -eu

# Configuration paths are injected by the deployment environment. Supplying
# explicit CLI arguments still works for local debugging and orchestrators
# which keep their own command contract.
if [ "$#" -gt 0 ]; then
    exec /usr/local/bin/logiksmith "$@"
fi
exec /usr/local/bin/logiksmith \
    --config "${LOGIKSMITH_CONFIG_PATH:-/config/local.toml}" \
    --automation "${LOGIKSMITH_AUTOMATION_PATH:-/config/automation.toml}"
