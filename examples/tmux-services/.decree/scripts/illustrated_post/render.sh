#!/usr/bin/env bash
# illustrated_post's render: queue a FLUX2 text-to-image workflow with the
# message body as its prompt, wait until ComfyUI has finished it (the next
# state's use_ollama ends ComfyUI, and must not cut a render short), and write
# the image's path to image.txt in the run directory.
set -euo pipefail
COMFY_URL="${COMFY_URL:-http://127.0.0.1:8188}"
# Reused by path from examples/text-to-media/; set this when the example is copied elsewhere.
COMFY_WORKFLOW="${COMFY_WORKFLOW:-$DECREE_PROJECT_ROOT/../text-to-media/workflows/image_flux2_text_landscape.json}"
COMFY_RENDER_TIMEOUT_S="${COMFY_RENDER_TIMEOUT_S:-1800}"
COMFYUI_DIR="${COMFYUI_DIR:-$HOME/ComfyUI}"   # its output/ holds the images

# The message body, after the frontmatter, is the prompt.
prompt=$(awk 'NR == 1 && /^---$/ { fm = 1; next } fm == 1 && /^---$/ { fm = 2; next } fm != 1' "$DECREE_MESSAGE")

# Node 6 is the prompt and node 9 the SaveImage node in the FLUX2 workflows.
payload="$DECREE_RUN_DIR/comfy-payload.json"
jq --arg text "$prompt" --arg prefix "decree_$DECREE_MESSAGE_ID" \
  '{prompt: (.prompt | .["6"].inputs.text = $text | .["9"].inputs.filename_prefix = $prefix)}' \
  "$COMFY_WORKFLOW" > "$payload"
queued=$(curl -fsS --max-time 30 -H 'content-type: application/json' -d "@$payload" "$COMFY_URL/prompt")
echo "$queued"
id=$(jq -r .prompt_id <<<"$queued")

# /history/<id> is {} until the prompt has finished.
for _ in $(seq "$COMFY_RENDER_TIMEOUT_S"); do
  history=$(curl -fsS --max-time 10 "$COMFY_URL/history/$id")
  if jq -e --arg id "$id" 'has($id)' <<<"$history" >/dev/null; then
    jq --arg id "$id" '.[$id].status' <<<"$history"
    if ! jq -e --arg id "$id" '.[$id].status.status_str == "success"' <<<"$history" >/dev/null; then
      echo "comfyui: prompt $id failed" >&2
      exit 1
    fi
    image=$(jq -r --arg id "$id" \
      '[.[$id].outputs[]?.images[]?] | first // empty | [.subfolder, .filename] | map(select(. != "")) | join("/")' \
      <<<"$history")
    if [ -z "$image" ]; then
      echo "comfyui: prompt $id saved no image" >&2
      exit 1
    fi
    echo "$COMFYUI_DIR/output/$image" > "$DECREE_RUN_DIR/image.txt"
    echo "wrote image.txt: $COMFYUI_DIR/output/$image"
    exit 0
  fi
  sleep 1
done
echo "comfyui: prompt $id did not finish within $COMFY_RENDER_TIMEOUT_S s" >&2
exit 1
