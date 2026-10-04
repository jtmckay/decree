#!/usr/bin/env bash
# onentry, before anything that ends ComfyUI (use_ollama): wait until ComfyUI's
# queue is empty, nothing running and nothing pending, so ending it cuts no job
# short. Then, while ComfyUI still holds its history, write what this run's
# prompts (comfy-prompts.txt) made to images.txt, one path per line, and fail if
# one of them failed or ComfyUI lost it.
set -euo pipefail
COMFY_URL="${COMFY_URL:-http://127.0.0.1:8188}"
COMFYUI_DIR="${COMFYUI_DIR:-$HOME/ComfyUI}"   # its output/ holds the images
COMFY_DRAIN_TIMEOUT_S="${COMFY_DRAIN_TIMEOUT_S:-1800}"
prompts="$DECREE_RUN_DIR/comfy-prompts.txt"

if ! curl -fsS --max-time 5 "$COMFY_URL/system_stats" >/dev/null 2>&1; then
  if [ -s "$prompts" ]; then
    echo "comfyui is not running, so the prompts this run queued ($(paste -sd ' ' "$prompts")) are lost" >&2
    exit 1
  fi
  echo "comfyui is not running: nothing to wait for"
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

[ -s "$prompts" ] || exit 0
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
