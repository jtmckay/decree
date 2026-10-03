#!/usr/bin/env bash
# Send the message body to the Chatterbox TTS server and save the WAV it
# returns in the run folder. Synthesis settings come from the machine's data,
# which a message overrides with params.
set -euo pipefail

# The message body: everything after the frontmatter, leading blank lines removed.
text=$(awk 'NR == 1 && /^---$/ { fm = 1; next } fm == 1 && /^---$/ { fm = 2; next } fm != 1' "${DECREE_MESSAGE}" \
  | sed '/\S/,$!d')
if [ -z "$text" ]; then
  echo "Error: message body is empty — nothing to speak" >&2
  exit 1
fi

payload=$(jq -n \
  --arg text "$text" \
  --argjson temperature "${DECREE_DATA_TEMPERATURE}" \
  --argjson exaggeration "${DECREE_DATA_EXAGGERATION}" \
  --argjson cfg_weight "${DECREE_DATA_CFG_WEIGHT}" \
  --argjson seed "${DECREE_DATA_SEED}" \
  --arg language "${DECREE_DATA_LANGUAGE}" \
  --arg voice_mode "${DECREE_DATA_VOICE_MODE}" \
  --argjson split_text "${DECREE_DATA_SPLIT_TEXT}" \
  --argjson chunk_size "${DECREE_DATA_CHUNK_SIZE}" \
  --arg output_format "${DECREE_DATA_OUTPUT_FORMAT}" \
  --arg predefined_voice_id "${DECREE_DATA_PREDEFINED_VOICE_ID}" \
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

wav="${DECREE_RUN_DIR}/${DECREE_DATA_FILENAME}.wav"
echo "=== Sending TTS request ==="
echo "Voice: ${DECREE_DATA_PREDEFINED_VOICE_ID} | Temp: ${DECREE_DATA_TEMPERATURE} | Exaggeration: ${DECREE_DATA_EXAGGERATION}"
echo "Text (first 100 chars): ${text:0:100}..."

curl -sf -X POST "${DECREE_DATA_TTS_HOST}/tts" \
  -H 'Content-Type: application/json' \
  -H 'Accept: */*' \
  --data-raw "$payload" \
  -o "$wav"
echo "Saved ${wav}"
