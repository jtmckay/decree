#!/usr/bin/env bash
# Generate
#
# Final step in the portrait chain. Reads prompt JSON, builds a ComfyUI
# SDXL txt2img workflow, submits via REST API, polls for completion,
# and downloads the result to $message_dir/portrait.png.
set -euo pipefail

# Parameters (decree injects these as env vars)
spec_file="${spec_file:-}"
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"

# Custom parameters
prompt_path="${prompt_path:-}"
comfyui_url="${comfyui_url:-http://127.0.0.1:8188}"
subject="${subject:-}"

# Read prompt data
if [ -z "$prompt_path" ] || [ ! -f "$prompt_path" ]; then
    echo "ERROR: prompt_path not set or file not found: ${prompt_path}" >&2
    exit 1
fi

POSITIVE=$(jq -r '.positive' "$prompt_path")
NEGATIVE=$(jq -r '.negative' "$prompt_path")
WIDTH=$(jq -r '.width // 1024' "$prompt_path")
HEIGHT=$(jq -r '.height // 1024' "$prompt_path")
CFG=$(jq -r '.cfg_scale // 7.0' "$prompt_path")
STEPS=$(jq -r '.steps // 30' "$prompt_path")
SAMPLER=$(jq -r '.sampler // "euler_ancestral"' "$prompt_path")
SEED=$RANDOM$RANDOM

echo "Generating portrait of ${subject}..."
echo "  Resolution: ${WIDTH}x${HEIGHT}"
echo "  Steps: ${STEPS}, CFG: ${CFG}, Sampler: ${SAMPLER}"

# Build ComfyUI API workflow payload
WORKFLOW=$(cat <<WFJSON
{
  "prompt": {
    "3": {
      "class_type": "KSampler",
      "inputs": {
        "seed": ${SEED},
        "steps": ${STEPS},
        "cfg": ${CFG},
        "sampler_name": "${SAMPLER}",
        "scheduler": "normal",
        "denoise": 1.0,
        "model": ["4", 0],
        "positive": ["6", 0],
        "negative": ["7", 0],
        "latent_image": ["5", 0]
      }
    },
    "4": {
      "class_type": "CheckpointLoaderSimple",
      "inputs": { "ckpt_name": "sd_xl_base_1.0.safetensors" }
    },
    "5": {
      "class_type": "EmptyLatentImage",
      "inputs": { "width": ${WIDTH}, "height": ${HEIGHT}, "batch_size": 1 }
    },
    "6": {
      "class_type": "CLIPTextEncode",
      "inputs": { "text": $(jq -Rs '.' <<< "$POSITIVE"), "clip": ["4", 1] }
    },
    "7": {
      "class_type": "CLIPTextEncode",
      "inputs": { "text": $(jq -Rs '.' <<< "$NEGATIVE"), "clip": ["4", 1] }
    },
    "8": {
      "class_type": "VAEDecode",
      "inputs": { "samples": ["3", 0], "vae": ["4", 2] }
    },
    "9": {
      "class_type": "SaveImage",
      "inputs": { "filename_prefix": "decree_portrait", "images": ["8", 0] }
    }
  }
}
WFJSON
)

# Submit to ComfyUI
echo "Submitting workflow to ${comfyui_url}..."
RESPONSE=$(curl -sf -X POST "${comfyui_url}/prompt" \
    -H "Content-Type: application/json" \
    -d "$WORKFLOW")

PROMPT_ID=$(echo "$RESPONSE" | jq -r '.prompt_id')
if [ -z "$PROMPT_ID" ] || [ "$PROMPT_ID" = "null" ]; then
    echo "ERROR: Failed to queue prompt. Response: ${RESPONSE}" >&2
    exit 1
fi

echo "Queued prompt: ${PROMPT_ID}"

# Poll for completion
MAX_WAIT=300
ELAPSED=0
while [ $ELAPSED -lt $MAX_WAIT ]; do
    HISTORY=$(curl -sf "${comfyui_url}/history/${PROMPT_ID}" 2>/dev/null || echo "{}")
    STATUS=$(echo "$HISTORY" | jq -r ".\"${PROMPT_ID}\".status.completed // empty")

    if [ "$STATUS" = "true" ]; then
        break
    fi

    sleep 2
    ELAPSED=$((ELAPSED + 2))
    echo "  Waiting... (${ELAPSED}s)"
done

if [ $ELAPSED -ge $MAX_WAIT ]; then
    echo "ERROR: Timed out waiting for generation after ${MAX_WAIT}s" >&2
    exit 1
fi

# Download the generated image
FILENAME=$(echo "$HISTORY" | jq -r ".\"${PROMPT_ID}\".outputs.\"9\".images[0].filename")
SUBFOLDER=$(echo "$HISTORY" | jq -r ".\"${PROMPT_ID}\".outputs.\"9\".images[0].subfolder // empty")

if [ -n "$SUBFOLDER" ]; then
    IMAGE_URL="${comfyui_url}/view?filename=${FILENAME}&subfolder=${SUBFOLDER}&type=output"
else
    IMAGE_URL="${comfyui_url}/view?filename=${FILENAME}&type=output"
fi

curl -sf -o "${message_dir}/portrait.png" "$IMAGE_URL"

echo "Portrait saved to ${message_dir}/portrait.png"
echo "Generation complete for: ${subject}"
