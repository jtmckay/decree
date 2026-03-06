#!/usr/bin/env bash
# Transcribe
#
# Transcribes an audio file using OpenAI Whisper. Reads the audio file
# path from the `input_file` frontmatter field, saves the transcription
# as a .txt file alongside the original.
#
# Custom parameters:
#   input_file  — (required) path to the audio file to transcribe
#   output_file — (optional) path for the output text file;
#                  defaults to the input file with a .txt extension
#   model       — (optional) whisper model to use; defaults to "large"
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

# Pre-check: verify whisper is installed
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then
    command -v whisper >/dev/null 2>&1 || { echo "whisper not found (pip install -U openai-whisper)" >&2; exit 1; }
    exit 0
fi

# Custom parameters (from frontmatter)
input_file="${input_file:-}"
output_file="${output_file:-}"
model="${model:-large}"

if [ -z "$input_file" ]; then
    echo "Error: input_file is required (set in message frontmatter)" >&2
    exit 1
fi

if [ ! -f "$input_file" ]; then
    echo "Error: input file not found: $input_file" >&2
    exit 1
fi

# Default output to input name with .txt extension
if [ -z "$output_file" ]; then
    output_file="${input_file%.*}.txt"
fi

output_dir="$(dirname "$output_file")"

echo "=== Transcribing ==="
echo "Input:  $input_file"
echo "Output: $output_file"
echo "Model:  $model"

# Run whisper — output to a temp directory so we control the final path
tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT

whisper "$input_file" \
    --model "$model" \
    --output_format txt \
    --output_dir "$tmpdir"

# Whisper names the output after the input file basename
whisper_output="$tmpdir/$(basename "${input_file%.*}").txt"

if [ ! -f "$whisper_output" ]; then
    echo "Error: whisper did not produce expected output at $whisper_output" >&2
    ls -la "$tmpdir" >&2
    exit 1
fi

# Move to the desired output path
mkdir -p "$output_dir"
mv "$whisper_output" "$output_file"

echo "=== Done ==="
echo "Transcription saved to: $output_file"
