#!/usr/bin/env bash
# onentry: ComfyUI's API is fire and forget, so jobs queued earlier (render)
# may still be running. Wait until ComfyUI's queue is empty, nothing running
# and nothing pending; then, while ComfyUI still holds its history, write what
# this run's prompts (comfy-prompts.txt) made to images.txt, failing if one of
# them failed or ComfyUI lost it; then end the comfyui tmux session, freeing
# the GPU. On a failure ComfyUI is left running, so nothing is lost.
set -euo pipefail
COMFY_URL="${COMFY_URL:-http://127.0.0.1:8188}"
COMFYUI_DIR="${COMFYUI_DIR:-$HOME/ComfyUI}"   # its output/ holds the images
COMFY_DRAIN_TIMEOUT_S="${COMFY_DRAIN_TIMEOUT_S:-1800}"
COMFYUI_SESSION="${COMFYUI_SESSION:-comfyui}"
COMFYUI_HEALTH="${COMFYUI_HEALTH:-$COMFY_URL/system_stats}"
source "$(dirname "${BASH_SOURCE[0]}")/tmux_service.sh"
prompts="$DECREE_RUN_DIR/comfy-prompts.txt"

# Write the images this run's prompts made to images.txt; exit 1 on a failed or lost prompt.
collect_images() {
  : > "$DECREE_RUN_DIR/images.txt"
  while read -r id; do
    history=$(curl -fsS --max-time 10 "$COMFY_URL/history/$id")
    if ! jq -e --arg id "$id" 'has($id)' <<<"$history" >/dev/null; then
      echo "comfyui: prompt $id is neither queued nor in its history: it was lost (did ComfyUI restart?)" >&2
      exit 1
    fi
    if ! jq -e --arg id "$id" '.[$id].status.status_str == "success"' <<<"$history" >/dev/null; then
      echo "comfyui: prompt $id failed: $(jq -c --arg id "$id" '.[$id].status' <<<"$history")" >&2
      exit 1
    fi
    jq -r --arg id "$id" --arg out "$COMFYUI_DIR/output" \
      '.[$id].outputs[]?.images[]? | [$out, .subfolder, .filename] | map(select(. != "")) | join("/")' \
      <<<"$history" >> "$DECREE_RUN_DIR/images.txt"
  done < "$prompts"
  if [ ! -s "$DECREE_RUN_DIR/images.txt" ]; then
    echo "comfyui: this run's prompts saved no image" >&2
    exit 1
  fi
  cat "$DECREE_RUN_DIR/images.txt"
}

if ! curl -fsS --max-time 5 "$COMFY_URL/system_stats" >/dev/null 2>&1; then
  if [ -s "$prompts" ]; then
    echo "comfyui is not running, so the prompts this run queued ($(paste -sd ' ' "$prompts")) are lost" >&2
    exit 1
  fi
  echo "comfyui is not running: nothing to wait for"
  end_session "$COMFYUI_SESSION" "$COMFYUI_HEALTH"
  exit 0
fi

# GET /queue: {"queue_running": [...], "queue_pending": [...]}; each job's prompt id is its second item.
for ((s = 0; ; s++)); do
  jobs=$(curl -fsS --max-time 10 "$COMFY_URL/queue" | jq '(.queue_running | length) + (.queue_pending | length)')
  [ "$jobs" -eq 0 ] && break
  if [ "$s" -ge "$COMFY_DRAIN_TIMEOUT_S" ]; then
    echo "comfyui still has $jobs job(s) after $COMFY_DRAIN_TIMEOUT_S s" >&2
    exit 1
  fi
  (( s % 30 == 0 )) && echo "waiting for comfyui: $jobs job(s) running or pending"
  sleep 1
done
echo "comfyui's queue is empty"

if [ -s "$prompts" ]; then
  collect_images
fi

end_session "$COMFYUI_SESSION" "$COMFYUI_HEALTH"
