#!/usr/bin/env bash
# comfy's await: poll GET /history/<prompt id> every COMFY_POLL_S seconds until
# ComfyUI lists the prompt there, which it does once the prompt finishes; keep
# the entry in runs/<id>/comfy-history.json for fetch, and fail if the prompt
# did not succeed. The state's timeout bounds the wait.
set -euo pipefail
COMFY_URL="${COMFY_URL:-http://127.0.0.1:8188}"
COMFY_POLL_S="${COMFY_POLL_S:-5}"
id=$(cat "${DECREE_RUN_DIR}/comfy-prompt-id")

echo "waiting for prompt ${id}"
until history=$(curl -fsS --max-time 30 "${COMFY_URL}/history/${id}") \
  && jq -e --arg id "${id}" 'has($id)' <<<"${history}" >/dev/null; do
  sleep "${COMFY_POLL_S}"
done
jq --arg id "${id}" '.[$id]' <<<"${history}" > "${DECREE_RUN_DIR}/comfy-history.json"

if ! jq -e '.status.status_str == "success"' "${DECREE_RUN_DIR}/comfy-history.json" >/dev/null; then
  echo "await: prompt ${id} failed: $(jq -c .status "${DECREE_RUN_DIR}/comfy-history.json")" >&2
  exit 1
fi
echo "prompt ${id} finished"
