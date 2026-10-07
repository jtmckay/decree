#!/usr/bin/env bash
# comfy's fetch: download each file the prompt saved (GET /view) to `output`
# plus the file's extension, or `output`-<n> plus its extension when there are
# several. Safe to repeat: each download goes to a .part file, then moves.
set -euo pipefail
COMFY_URL="${COMFY_URL:-http://127.0.0.1:8188}"
history="${DECREE_RUN_DIR}/comfy-history.json"
output="${DECREE_PROJECT_ROOT}/${DECREE_DATA_OUTPUT}"

# Each node's outputs map a kind (images, gifs, ...) to a list of
# {filename, subfolder, type}; "output" files are saved, "temp" are previews.
mapfile -t files < <(jq -c '.outputs[][] | arrays | .[] | objects
  | select(.filename and .type == "output")' "${history}")
if [ "${#files[@]}" -eq 0 ]; then
  echo "fetch: the prompt saved no file" >&2
  exit 1
fi

mkdir -p "$(dirname "${output}")"
n=0
for file in "${files[@]}"; do
  n=$((n + 1))
  filename=$(jq -r .filename <<<"${file}")
  if [ "${#files[@]}" -eq 1 ]; then
    dest="${output}.${filename##*.}"
  else
    dest="${output}-${n}.${filename##*.}"
  fi
  curl -fsS --max-time 600 -G "${COMFY_URL}/view" \
    --data-urlencode "filename=${filename}" \
    --data-urlencode "subfolder=$(jq -r '.subfolder // ""' <<<"${file}")" \
    --data-urlencode "type=output" \
    -o "${dest}.part"
  mv "${dest}.part" "${dest}"
  echo "saved ${dest#"${DECREE_PROJECT_ROOT}/"}"
done
