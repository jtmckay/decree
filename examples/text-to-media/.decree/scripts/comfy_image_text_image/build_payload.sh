#!/usr/bin/env bash
# comfy_image_text_image's build_payload: patch the FLUX2 text+image workflow
# with the prompt (the message body), the reference image, the size and the
# output prefix, and save it as runs/<id>/comfy-payload.json for queue_prompt.
set -euo pipefail

# Align to the nearest multiple of 16 (required by diffusion models).
align16() { echo $(( (($1 + 8) / 16) * 16 )); }
width=$(align16 "${DECREE_DATA_WIDTH}")
height=$(align16 "${DECREE_DATA_HEIGHT}")

# The message body is the prompt: everything after the frontmatter, leading
# blank lines removed.
prompt_text=$(awk 'NR == 1 && /^---$/ { fm = 1; next } fm == 1 && /^---$/ { fm = 2; next } fm != 1' "${DECREE_MESSAGE}" \
  | sed '/\S/,$!d')

if [ -z "$prompt_text" ]; then
  echo "Error: Prompt text is empty" >&2
  exit 1
fi
if [ -z "${DECREE_DATA_INPUT_IMAGE}" ]; then
  echo "Error: input_image is required (set it in the message's params)" >&2
  exit 1
fi
if [ -z "${DECREE_DATA_OUTPUT_PREFIX}" ]; then
  echo "Error: output_prefix is required (set it in the message's params)" >&2
  exit 1
fi

template="${DECREE_PROJECT_ROOT}/workflows/image_flux2_text_image.json"

echo "=== ComfyUI FLUX2 Image (Text + Reference Image) ==="
echo "  Prompt:   ${prompt_text:0:80}..."
echo "  Image:    ${DECREE_DATA_INPUT_IMAGE}"
echo "  Size:     ${width}x${height}"
echo "  Output:   ${DECREE_DATA_OUTPUT_PREFIX}"
echo "  API:      ${DECREE_DATA_API_URL}"

jq \
  --arg text "$prompt_text" \
  --arg image "${DECREE_DATA_INPUT_IMAGE}" \
  --argjson width "$width" \
  --argjson height "$height" \
  --arg output "${DECREE_DATA_OUTPUT_PREFIX}" \
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
  ' "$template" > "${DECREE_RUN_DIR}/comfy-payload.json"

echo "=== Payload verification ==="
jq '{
  text_preview: (.prompt["6"].inputs.text[:60] + "..."),
  input_image: .prompt["46"].inputs.image,
  latent_size: "\(.prompt["47"].inputs.width)x\(.prompt["47"].inputs.height)",
  output: .prompt["9"].inputs.filename_prefix
}' "${DECREE_RUN_DIR}/comfy-payload.json"
