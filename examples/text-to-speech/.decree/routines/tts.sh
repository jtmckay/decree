#!/usr/bin/env bash
# TTS
#
# Text-to-speech routine. Sends text to a local Chatterbox TTS server
# and saves the resulting audio as MP3. The text to speak comes from
# the message body. Synthesis parameters can be overridden via
# frontmatter fields.
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
    command -v ffmpeg >/dev/null 2>&1 || { echo "ffmpeg not found" >&2; exit 1; }
    command -v jq >/dev/null 2>&1 || { echo "jq not found" >&2; exit 1; }
    exit 0
fi

# Custom parameters (from frontmatter, override via env vars)
filename="${filename:-}"
output_dir="${output_dir:-${PWD}/output}"
temperature="${temperature:-0.8}"
exaggeration="${exaggeration:-1.3}"
cfg_weight="${cfg_weight:-0.5}"
seed="${seed:-3000}"
language="${language:-en}"
voice_mode="${voice_mode:-predefined}"
split_text="${split_text:-true}"
chunk_size="${chunk_size:-240}"
output_format="${output_format:-wav}"
predefined_voice_id="${predefined_voice_id:-Emily.wav}"
tts_host="${tts_host:-http://localhost:8004}"

if [ -z "$filename" ]; then
    echo "Error: filename is required (set in message frontmatter)" >&2
    exit 1
fi

# Strip YAML frontmatter to get the text body
text=$(sed '1{/^---$/!q}; 1,/^---$/d' "$message_file")

if [ -z "$text" ]; then
    echo "Error: message body is empty — nothing to speak" >&2
    exit 1
fi

# Build the JSON payload
payload=$(jq -n \
    --arg text "$text" \
    --argjson temperature "$temperature" \
    --argjson exaggeration "$exaggeration" \
    --argjson cfg_weight "$cfg_weight" \
    --argjson seed "$seed" \
    --arg language "$language" \
    --arg voice_mode "$voice_mode" \
    --argjson split_text "$split_text" \
    --argjson chunk_size "$chunk_size" \
    --arg output_format "$output_format" \
    --arg predefined_voice_id "$predefined_voice_id" \
    '{
      text: $text,
      temperature: $temperature,
      exaggeration: $exaggeration,
      cfg_weight: $cfg_weight,
      seed: $seed,
      language: $language,
      voice_mode: $voice_mode,
      split_text: $split_text,
      chunk_size: $chunk_size,
      output_format: $output_format,
      predefined_voice_id: $predefined_voice_id
    }')

tmp_dir="${output_dir}/tmp"
tmp_wav="${tmp_dir}/${filename}.wav"
output_file="${output_dir}/${filename}.mp3"
mkdir -p "$tmp_dir" "$(dirname "$output_file")"

echo "=== Sending TTS request ==="
echo "Voice: ${predefined_voice_id} | Temp: ${temperature} | Exaggeration: ${exaggeration}"
echo "Text (first 100 chars): ${text:0:100}..."

curl -s -X POST "${tts_host}/tts" \
    -H 'Content-Type: application/json' \
    -H 'Accept: */*' \
    --data-raw "$payload" \
    -o "$tmp_wav"

echo "=== Converting WAV to MP3 ==="
if ffmpeg -y -i "$tmp_wav" -codec:a libmp3lame -b:a 192k "$output_file" 2>/dev/null; then
    rm "$tmp_wav"
    rmdir "$tmp_dir" 2>/dev/null || true
    echo "=== TTS complete ==="
    echo "Output saved to: ${output_file}"
else
    echo "Error: ffmpeg conversion failed. Keeping WAV at: ${tmp_wav}" >&2
    exit 1
fi
