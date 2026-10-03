#!/usr/bin/env bash
# Fail fast if a tool the ComfyUI machines need is missing.
set -euo pipefail
for tool in curl jq; do
  command -v "$tool" >/dev/null || { echo "$tool not found" >&2; exit 1; }
done
echo "tools ok"
