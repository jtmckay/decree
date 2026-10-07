#!/usr/bin/env bash
# comfy's build: patch workflows/<method>.json with the message and its params,
# and save it as runs/<id>/comfy-payload.json for submit. Every workflow is
# patched the same way, by node type, so a new method is a new workflow file:
#   the prompt (the message body)  every CLIPTextEncode no `negative` input uses
#   width, height (0 keeps)        every node with numeric width and height
#   seed (-1 keeps)                every node with a numeric seed or noise_seed
#   the file name prefix           every node with a filename_prefix
# input_image is required when the workflow has a LoadImage node; submit
# uploads it and points LoadImage at it.
set -euo pipefail
workflows="${DECREE_PROJECT_ROOT}/workflows"
method="${DECREE_DATA_METHOD}"

methods() {
  local f
  for f in "${workflows}"/*.json; do basename "${f}" .json; done | paste -sd ',' | sed 's/,/, /g'
}
fail() {
  echo "build: $1" >&2
  exit 1
}

if [ -z "${method}" ]; then
  fail "method is required; the methods are: $(methods)"
fi
workflow="${workflows}/${method}.json"
if [[ "${method}" == */* ]] || [ ! -f "${workflow}" ]; then
  fail "unknown method '${method}'; the methods are: $(methods)"
fi
[ -n "${DECREE_DATA_OUTPUT}" ] || fail "output is required: a repo path without extension"

# The message body is the prompt: everything after the frontmatter, leading
# blank lines removed.
prompt=$(awk 'NR == 1 && /^---$/ { fm = 1; next } fm == 1 && /^---$/ { fm = 2; next } fm != 1' "${DECREE_MESSAGE}" \
  | sed '/\S/,$!d')
[ -n "${prompt}" ] || fail "the message body, the prompt, is empty"

if jq -e '[.prompt[] | select(.class_type == "LoadImage")] | length > 0' "${workflow}" >/dev/null; then
  [ -n "${DECREE_DATA_INPUT_IMAGE}" ] || fail "input_image is required by method ${method}"
  [ -f "${DECREE_PROJECT_ROOT}/${DECREE_DATA_INPUT_IMAGE}" ] \
    || fail "input_image ${DECREE_DATA_INPUT_IMAGE} is not a file in the repo"
fi

# Diffusion models need sizes in multiples of 16: round to the nearest.
align16() { echo $(( (($1 + 8) / 16) * 16 )); }
width=0
height=0
[ "${DECREE_DATA_WIDTH}" -le 0 ] || width=$(align16 "${DECREE_DATA_WIDTH}")
[ "${DECREE_DATA_HEIGHT}" -le 0 ] || height=$(align16 "${DECREE_DATA_HEIGHT}")

echo "=== ${method} ==="
echo "  prompt:  ${prompt:0:80}"
echo "  output:  ${DECREE_DATA_OUTPUT}"
[ -z "${DECREE_DATA_INPUT_IMAGE}" ] || echo "  image:   ${DECREE_DATA_INPUT_IMAGE}"
[ "${width}${height}" = 00 ] || echo "  size:    ${width}x${height} (0 keeps the workflow's)"
[ "${DECREE_DATA_SEED}" -lt 0 ] || echo "  seed:    ${DECREE_DATA_SEED}"

jq \
  --arg text "${prompt}" \
  --argjson width "${width}" \
  --argjson height "${height}" \
  --argjson seed "${DECREE_DATA_SEED}" \
  --arg prefix "decree_${DECREE_MESSAGE_ID}" \
  '
  def number($key): (.inputs[$key] | type) == "number";
  [.prompt[] | .inputs.negative? | arrays | .[0]] as $negative
  | {prompt: (.prompt | with_entries(
      (if .value.class_type == "CLIPTextEncode" and (.key | IN($negative[]) | not)
       then .value.inputs.text = $text else . end)
      | (if $width > 0 and (.value | number("width")) then .value.inputs.width = $width else . end)
      | (if $height > 0 and (.value | number("height")) then .value.inputs.height = $height else . end)
      | (if $seed >= 0 and (.value | number("seed")) then .value.inputs.seed = $seed else . end)
      | (if $seed >= 0 and (.value | number("noise_seed")) then .value.inputs.noise_seed = $seed else . end)
      | (if (.value.inputs.filename_prefix | type) == "string" then .value.inputs.filename_prefix = $prefix else . end)
    ))}
  ' "${workflow}" > "${DECREE_RUN_DIR}/comfy-payload.json"
echo "wrote comfy-payload.json"
