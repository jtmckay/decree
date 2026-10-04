#!/usr/bin/env bash
# onentry: use ComfyUI if it already answers, whoever runs it; otherwise use
# the comfyui tmux session, or start ComfyUI's main.py in a new one, and wait
# until it answers. It does not free the GPU: put without_ollama before it.
set -euo pipefail
COMFYUI_SESSION="${COMFYUI_SESSION:-comfyui}"
COMFYUI_DIR="${COMFYUI_DIR:-$HOME/ComfyUI}"
COMFYUI_PYTHON="${COMFYUI_PYTHON:-python3}"   # or ComfyUI's venv, e.g. $COMFYUI_DIR/venv/bin/python
COMFYUI_HOST="${COMFYUI_HOST:-127.0.0.1}"
COMFYUI_PORT="${COMFYUI_PORT:-8188}"
COMFYUI_COMMAND="${COMFYUI_COMMAND:-cd $(printf '%q' "$COMFYUI_DIR") && $COMFYUI_PYTHON main.py --listen $COMFYUI_HOST --port $COMFYUI_PORT}"
COMFYUI_HEALTH="${COMFYUI_HEALTH:-http://$COMFYUI_HOST:$COMFYUI_PORT/system_stats}"
COMFYUI_START_TIMEOUT_S="${COMFYUI_START_TIMEOUT_S:-180}"
source "$(dirname "${BASH_SOURCE[0]}")/tmux_service.sh"

if answers "$COMFYUI_HEALTH"; then
  echo "comfyui already answers at $COMFYUI_HEALTH: using it, whoever runs it"
  exit 0
fi
ensure_session "$COMFYUI_SESSION" "$COMFYUI_COMMAND"
wait_until_up "$COMFYUI_HEALTH" "$COMFYUI_START_TIMEOUT_S" || {
  echo "comfyui did not start; see why with: tmux attach -t $COMFYUI_SESSION" >&2
  exit 1
}
