#!/usr/bin/env bash
# Fail fast if a tool rust_develop needs is missing.
set -euo pipefail
for tool in claude cargo; do
  command -v "${tool}" >/dev/null || { echo "${tool} not found" >&2; exit 1; }
done
echo "tools ok"
