#!/usr/bin/env bash
# onentry: use the ollama tmux session, or start `ollama serve` in a new one,
# and wait until it answers. It does not free the GPU: put without_comfy_wait
# or without_comfy_no_wait before it.
set -euo pipefail
OLLAMA_SESSION="${OLLAMA_SESSION:-ollama}"
OLLAMA_COMMAND="${OLLAMA_COMMAND:-ollama serve}"
OLLAMA_HEALTH="${OLLAMA_HEALTH:-http://127.0.0.1:11434/api/version}"
OLLAMA_START_TIMEOUT_S="${OLLAMA_START_TIMEOUT_S:-60}"
source "$(dirname "${BASH_SOURCE[0]}")/tmux_service.sh"

ensure_session "$OLLAMA_SESSION" "$OLLAMA_COMMAND"
wait_until_up "$OLLAMA_HEALTH" "$OLLAMA_START_TIMEOUT_S" || {
  echo "ollama did not start; see why with: tmux attach -t $OLLAMA_SESSION" >&2
  exit 1
}
