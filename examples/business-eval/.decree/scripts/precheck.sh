#!/usr/bin/env bash
# Fail fast if claude is not installed.
set -euo pipefail
command -v claude >/dev/null || { echo "claude not found" >&2; exit 1; }
echo "claude ok"
