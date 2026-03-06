#!/usr/bin/env bash
# ComfyUI Image (Text + Reference Image)
#
# Generates an image via the ComfyUI FLUX2 workflow using a text prompt
# and a reference image. The prompt text is read from the message body.
#
# Custom parameters:
#   width          — Image width  (default: 1024)
#   height         — Image height (default: 1024)
#   input_image    — Reference image filename as known to ComfyUI (required)
#   output_prefix  — Output filename prefix (required)
#   api_url        — ComfyUI API URL (default: http://127.0.0.1:8288/api/prompt)
set -euo pipefail

# --- Standard Environment Variables ---
# message_file  - Path to message.md in the run directory
# message_id    - Full message ID (e.g., D0001-1432-01-add-auth-0)
# message_dir   - Run directory path (contains logs from prior attempts)
# chain         - Chain ID (D<NNNN>-HHmm-<name>)
# seq           - Sequence number in chain
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"

# Pre-check: verify required tools are available
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then
    command -v curl >/dev/null 2>&1 || { echo "curl not found" >&2; exit 1; }
    command -v jq >/dev/null 2>&1 || { echo "jq not found" >&2; exit 1; }
    exit 0
fi

# Custom parameters
width="${width:-1024}"
height="${height:-1024}"
input_image="${input_image:-}"
output_prefix="${output_prefix:-}"
api_url="${api_url:-http://127.0.0.1:8288/api/prompt}"

# Align to nearest multiple of 16 (required by diffusion models)
align16() { echo $(( (($1 + 8) / 16) * 16 )); }
width=$(align16 "$width")
height=$(align16 "$height")

# Strip YAML frontmatter and leading blank lines to get prompt text
read_body() {
    awk 'BEGIN{fm=0} /^---$/{fm++; next} fm>=2' "$1" | sed '/\S/,$!d'
}

prompt_text=$(read_body "$message_file")

if [ -z "$prompt_text" ]; then
    echo "Error: Prompt text is empty" >&2
    exit 1
fi
if [ -z "$input_image" ]; then
    echo "Error: input_image is required (set in message frontmatter)" >&2
    exit 1
fi
if [ -z "$output_prefix" ]; then
    echo "Error: output_prefix is required (set in message frontmatter)" >&2
    exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TEMPLATE="$SCRIPT_DIR/../../workflows/image_flux2_text_image.json"

if [ ! -f "$TEMPLATE" ]; then
    echo "Error: Template not found at $TEMPLATE" >&2
    exit 1
fi

echo "=== ComfyUI FLUX2 Image (Text + Reference Image) ==="
echo "  Prompt:   ${prompt_text:0:80}..."
echo "  Image:    $input_image"
echo "  Size:     ${width}x${height}"
echo "  Output:   $output_prefix"
echo "  API:      $api_url"

PAYLOAD=$(jq \
    --arg text "$prompt_text" \
    --arg image "$input_image" \
    --argjson width "$width" \
    --argjson height "$height" \
    --arg output "$output_prefix" \
    '
    .prompt["6"].inputs.text = $text |
    .prompt["46"].inputs.image = $image |
    .prompt["47"].inputs.width = $width |
    .prompt["47"].inputs.height = $height |
    .prompt["48"].inputs.width = $width |
    .prompt["48"].inputs.height = $height |
    .prompt["9"].inputs.filename_prefix = $output |
    (.extra_data.extra_pnginfo.workflow.nodes[] | select(.id == 6)).widgets_values[0] = $text |
    (.extra_data.extra_pnginfo.workflow.nodes[] | select(.id == 46)).widgets_values[0] = $image |
    (.extra_data.extra_pnginfo.workflow.nodes[] | select(.id == 47)).widgets_values = [$width, $height, 1] |
    (.extra_data.extra_pnginfo.workflow.nodes[] | select(.id == 48)).widgets_values = [20, $width, $height] |
    (.extra_data.extra_pnginfo.workflow.nodes[] | select(.id == 50)).widgets_values[0] = $width |
    (.extra_data.extra_pnginfo.workflow.nodes[] | select(.id == 51)).widgets_values[0] = $height |
    (.extra_data.extra_pnginfo.workflow.nodes[] | select(.id == 9)).widgets_values[0] = $output
    ' "$TEMPLATE")

if [ -n "$message_dir" ] && [ -d "$message_dir" ]; then
    echo "$PAYLOAD" > "${message_dir}/comfy-payload.json"
fi

echo "=== Payload verification ==="
echo "$PAYLOAD" | jq '{
  text_preview: (.prompt["6"].inputs.text[:60] + "..."),
  input_image: .prompt["46"].inputs.image,
  latent_size: "\(.prompt["47"].inputs.width)x\(.prompt["47"].inputs.height)",
  output: .prompt["9"].inputs.filename_prefix
}'

RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "$api_url" \
    -H 'Content-Type: application/json' \
    --data-raw "$PAYLOAD")

HTTP_CODE=$(echo "$RESPONSE" | tail -1)
BODY=$(echo "$RESPONSE" | sed '$d')

echo "=== Response (HTTP $HTTP_CODE) ==="
echo "$BODY"

if [ -n "$message_dir" ] && [ -d "$message_dir" ]; then
    echo "$BODY" > "${message_dir}/comfy-response.json"
fi

if [ "$HTTP_CODE" -ge 200 ] && [ "$HTTP_CODE" -lt 300 ]; then
    exit 0
else
    exit 1
fi
