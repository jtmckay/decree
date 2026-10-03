#!/usr/bin/env bash
# Fail fast if whisper is not installed or the input file is missing.
set -euo pipefail
command -v whisper >/dev/null || { echo "whisper not found (pip install -U openai-whisper)" >&2; exit 1; }
if [ -z "${DECREE_DATA_INPUT_FILE}" ]; then
  echo "Error: input_file is required (set it in the message's params)" >&2
  exit 1
fi
if [ ! -f "${DECREE_DATA_INPUT_FILE}" ]; then
  echo "Error: input file not found: ${DECREE_DATA_INPUT_FILE}" >&2
  exit 1
fi
echo "whisper ok, input ${DECREE_DATA_INPUT_FILE}"
