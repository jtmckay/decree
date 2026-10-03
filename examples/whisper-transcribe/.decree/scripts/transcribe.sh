#!/usr/bin/env bash
# Transcribe input_file with Whisper and save the text as output_file
# (default: the input file with a .txt extension).
set -euo pipefail
input_file="${DECREE_DATA_INPUT_FILE}"
output_file="${DECREE_DATA_OUTPUT_FILE:-${input_file%.*}.txt}"
model="${DECREE_DATA_MODEL}"

echo "=== Transcribing ==="
echo "Input:  $input_file"
echo "Output: $output_file"
echo "Model:  $model"

# Whisper writes into a directory and names the file after the input, so
# write to a temp directory and move the result to output_file.
tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT

whisper "$input_file" \
  --model "$model" \
  --output_format txt \
  --output_dir "$tmpdir"

whisper_output="$tmpdir/$(basename "${input_file%.*}").txt"
if [ ! -f "$whisper_output" ]; then
  echo "Error: whisper did not produce expected output at $whisper_output" >&2
  ls -la "$tmpdir" >&2
  exit 1
fi

mkdir -p "$(dirname "$output_file")"
mv "$whisper_output" "$output_file"

echo "=== Done ==="
echo "Transcription saved to: $output_file"
