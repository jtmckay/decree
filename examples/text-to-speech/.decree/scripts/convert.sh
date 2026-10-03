#!/usr/bin/env bash
# Convert the WAV that synthesize saved in the run folder to an MP3 in output_dir.
set -euo pipefail
wav="${DECREE_RUN_DIR}/${DECREE_DATA_FILENAME}.wav"
output_file="${DECREE_DATA_OUTPUT_DIR}/${DECREE_DATA_FILENAME}.mp3"
mkdir -p "$(dirname "$output_file")"

echo "=== Converting WAV to MP3 ==="
if ! ffmpeg -y -i "$wav" -codec:a libmp3lame -b:a 192k "$output_file" 2>/dev/null; then
  echo "Error: ffmpeg conversion failed. Keeping WAV at: ${wav}" >&2
  exit 1
fi
rm "$wav"
echo "=== TTS complete ==="
echo "Output saved to: ${output_file}"
