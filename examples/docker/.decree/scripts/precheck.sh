#!/usr/bin/env bash
# Fail fast if opencode is not installed (set DECREE_AI=opencode in docker-compose.yml).
set -euo pipefail
command -v opencode >/dev/null || { echo "opencode not found" >&2; exit 1; }
echo "opencode ok"
