#!/usr/bin/env bash
# Fail fast if the AI CLI develop needs is missing.
set -euo pipefail
for tool in {ai}; do
  command -v "${tool}" >/dev/null || { echo "${tool} not found" >&2; exit 1; }
done
echo "tools ok"
