#!/usr/bin/env bash
# Shared invoke: fail fast if a tool is missing. 0.4's DECREE_PRE_CHECK mode
# is now just the first state of a machine.
set -euo pipefail
for tool in claude cargo git; do
  command -v "$tool" >/dev/null || { echo "$tool not found" >&2; exit 1; }
done
echo "tools ok"
