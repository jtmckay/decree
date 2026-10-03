#!/usr/bin/env bash
# comfy_video_i2v's build_payload: patch the WAN2.2 image-to-video workflow
# with the prompt (the message body), the first-frame image, the size and the
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

template="${DECREE_PROJECT_ROOT}/workflows/video_i2v_wan2.2_14B_long.json"

echo "=== ComfyUI WAN2.2 Video (Image-to-Video) ==="
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
  .prompt["93"].inputs.text = $text |
  .prompt["97"].inputs.image = $image |
  .prompt["98"].inputs.width = $width |
  .prompt["98"].inputs.height = $height |
  .prompt["108"].inputs.filename_prefix = $output |
  (.extra_data.extra_pnginfo.workflow.nodes[] | select(.id == 93)).widgets_values[0] = $text |
  (.extra_data.extra_pnginfo.workflow.nodes[] | select(.id == 97)).widgets_values[0] = $image |
  (.extra_data.extra_pnginfo.workflow.nodes[] | select(.id == 98)).widgets_values[0] = $width |
  (.extra_data.extra_pnginfo.workflow.nodes[] | select(.id == 98)).widgets_values[1] = $height |
  (.extra_data.extra_pnginfo.workflow.nodes[] | select(.id == 108)).widgets_values[0] = $output
  ' "$template" > "${DECREE_RUN_DIR}/comfy-payload.json"

echo "=== Payload verification ==="
jq '{
  text_preview: (.prompt["93"].inputs.text[:60] + "..."),
  input_image: .prompt["97"].inputs.image,
  video_size: "\(.prompt["98"].inputs.width)x\(.prompt["98"].inputs.height)",
  output: .prompt["108"].inputs.filename_prefix
}' "${DECREE_RUN_DIR}/comfy-payload.json"
