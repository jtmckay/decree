#!/usr/bin/env bash
# comfy's root onentry: fail fast if a tool the scripts need is missing.
set -euo pipefail
for tool in curl jq; do
  command -v "$tool" >/dev/null || { echo "$tool not found" >&2; exit 1; }
done
echo "tools ok"
