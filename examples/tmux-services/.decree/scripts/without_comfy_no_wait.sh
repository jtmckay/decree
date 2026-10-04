#!/usr/bin/env bash
# onentry: free the GPU from ComfyUI now; the server keeps running. Clears the
# pending queue (POST /queue {"clear": true}), stops the running job (POST
# /interrupt), then asks ComfyUI to unload its models and empty its cache (POST
# /free), which its worker does between jobs. Waits until GET /system_stats
# reports at most COMFY_RESERVED_MAX_MB reserved by PyTorch (torch_vram_total)
# on every device. The CUDA context, a few hundred MB, stays. Jobs still queued
# or running are lost: use it when they no longer matter, and
# without_comfy_wait when they do.
set -euo pipefail
COMFY_URL="${COMFY_URL:-http://127.0.0.1:8188}"
COMFY_RESERVED_MAX_MB="${COMFY_RESERVED_MAX_MB:-1024}"
COMFY_UNLOAD_TIMEOUT_S="${COMFY_UNLOAD_TIMEOUT_S:-60}"

post() {
  curl -fsS --max-time 10 -H 'content-type: application/json' -d "$2" "$COMFY_URL$1" >/dev/null
}

# Each device's name and the MB PyTorch has reserved on it, one per line; fails
# if ComfyUI does not answer.
reserved() {
  local stats
  stats=$(curl -fsS --max-time 5 "$COMFY_URL/system_stats" 2>/dev/null) || return 1
  jq -r '.devices[] | "\(.name): \(.torch_vram_total / 1048576 | floor) MB"' <<<"$stats"
}

if ! reserved >/dev/null; then
  echo "comfyui does not answer at $COMFY_URL: nothing to unload"
  exit 0
fi
post /queue '{"clear": true}'
post /interrupt '{}'
post /free '{"unload_models": true, "free_memory": true}'
echo "comfyui: cleared the queue, interrupted the running job, asked it to unload its models"

for ((s = 0; ; s++)); do
  # A ComfyUI that stopped answering holds no memory either.
  devices=$(reserved) || devices=""
  over=$(awk -F': ' -v max="$COMFY_RESERVED_MAX_MB" '$NF + 0 > max' <<<"$devices" | paste -sd ',' | sed 's/,/, /g')
  [ -z "$over" ] && break
  if [ "$s" -ge "$COMFY_UNLOAD_TIMEOUT_S" ]; then
    echo "comfyui: still $over reserved (more than $COMFY_RESERVED_MAX_MB MB) $COMFY_UNLOAD_TIMEOUT_S s after /free" >&2
    exit 1
  fi
  sleep 1
done
echo "comfyui has unloaded its models; its server keeps running"
