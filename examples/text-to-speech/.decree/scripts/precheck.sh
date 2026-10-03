#!/usr/bin/env bash
# Fail fast if a tool the tts machine needs is missing.
set -euo pipefail
for tool in curl ffmpeg jq; do
  command -v "$tool" >/dev/null || { echo "$tool not found" >&2; exit 1; }
done
if [ -z "${DECREE_DATA_FILENAME}" ]; then
  echo "Error: filename is required (set it in the message's params)" >&2
  exit 1
fi
echo "tools ok"
