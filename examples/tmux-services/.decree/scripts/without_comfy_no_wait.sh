#!/usr/bin/env bash
# onentry: end the comfyui tmux session now, and wait until ComfyUI stops
# answering, freeing the GPU. Jobs still queued or running are lost: use it
# when they no longer matter, and without_comfy_wait when they do.
set -euo pipefail
COMFYUI_SESSION="${COMFYUI_SESSION:-comfyui}"
COMFYUI_HEALTH="${COMFYUI_HEALTH:-http://127.0.0.1:8188/system_stats}"
source "$(dirname "${BASH_SOURCE[0]}")/tmux_service.sh"

end_session "$COMFYUI_SESSION" "$COMFYUI_HEALTH"
