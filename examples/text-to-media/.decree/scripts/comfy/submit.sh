#!/usr/bin/env bash
# comfy's submit: upload input_image, if any, and point every LoadImage node at
# it (POST /upload/image); queue the payload (POST /prompt), and keep the
# prompt id in runs/<id>/comfy-prompt-id for await. Safe to repeat: the upload
# overwrites, and a repeat queues the same payload.
set -euo pipefail
COMFY_URL="${COMFY_URL:-http://127.0.0.1:8188}"
payload="${DECREE_RUN_DIR}/comfy-payload.json"

if [ -n "${DECREE_DATA_INPUT_IMAGE}" ]; then
  uploaded=$(curl -fsS --max-time 60 -F "image=@${DECREE_PROJECT_ROOT}/${DECREE_DATA_INPUT_IMAGE}" \
    -F overwrite=true "${COMFY_URL}/upload/image")
  echo "uploaded: ${uploaded}"
  # ComfyUI replies {"name", "subfolder", "type": "input"}; LoadImage takes subfolder/name.
  image=$(jq -r '[.subfolder, .name] | map(select(. != "" and . != null)) | join("/")' <<<"${uploaded}")
  jq --arg image "${image}" \
    '.prompt |= map_values(if .class_type == "LoadImage" then .inputs.image = $image else . end)' \
    "${payload}" > "${payload}.tmp"
  mv "${payload}.tmp" "${payload}"
fi

queued=$(curl -sS --fail-with-body --max-time 30 -H 'content-type: application/json' \
  -d "@${payload}" "${COMFY_URL}/prompt")
echo "queued: ${queued}"
id=$(jq -r '.prompt_id // empty' <<<"${queued}")
if [ -z "${id}" ]; then
  echo "submit: ComfyUI's reply has no prompt_id" >&2
  exit 1
fi
echo "${id}" > "${DECREE_RUN_DIR}/comfy-prompt-id"
echo "prompt ${id}"
