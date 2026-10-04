#!/usr/bin/env bash
# onentry: Ollama and ComfyUI share the GPU. End the comfyui tmux session, then
# use the ollama tmux session, or start `ollama serve` in a new one, and wait
# until it answers.
set -euo pipefail
OLLAMA_SESSION="${OLLAMA_SESSION:-ollama}"
OLLAMA_COMMAND="${OLLAMA_COMMAND:-ollama serve}"
OLLAMA_HEALTH="${OLLAMA_HEALTH:-http://127.0.0.1:11434/api/version}"
OLLAMA_START_TIMEOUT_S="${OLLAMA_START_TIMEOUT_S:-60}"
COMFYUI_SESSION="${COMFYUI_SESSION:-comfyui}"
COMFYUI_HEALTH="${COMFYUI_HEALTH:-http://127.0.0.1:8188/system_stats}"
source "$(dirname "${BASH_SOURCE[0]}")/tmux_service.sh"

end_session "$COMFYUI_SESSION" "$COMFYUI_HEALTH"
ensure_session "$OLLAMA_SESSION" "$OLLAMA_COMMAND"
wait_until_up "$OLLAMA_HEALTH" "$OLLAMA_START_TIMEOUT_S" || {
  echo "ollama did not start; see why with: tmux attach -t $OLLAMA_SESSION" >&2
  exit 1
}
