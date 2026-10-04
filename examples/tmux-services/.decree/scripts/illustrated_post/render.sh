#!/usr/bin/env bash
# illustrated_post's render: queue a FLUX2 text-to-image workflow with the
# message body as its prompt, and return at once. ComfyUI renders in the
# background; the prompt id goes to comfy-prompts.txt in the run directory, and
# without_comfy_wait collects the image before ComfyUI ends.
set -euo pipefail
COMFY_URL="${COMFY_URL:-http://127.0.0.1:8188}"
# Reused by path from examples/text-to-media/; set this when the example is copied elsewhere.
COMFY_WORKFLOW="${COMFY_WORKFLOW:-$DECREE_PROJECT_ROOT/../text-to-media/workflows/image_flux2_text_landscape.json}"

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
echo "$id" >> "$DECREE_RUN_DIR/comfy-prompts.txt"
echo "queued prompt $id; without_comfy_wait collects its image"
