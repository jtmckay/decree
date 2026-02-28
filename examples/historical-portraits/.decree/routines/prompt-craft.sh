#!/usr/bin/env bash
# Prompt Craft
#
# Second step in the portrait chain. Reads research JSON and crafts
# SDXL-optimized positive and negative prompts. Writes prompt.json
# to $message_dir, then chains to generate.
set -euo pipefail

# Parameters (decree injects these as env vars)
spec_file="${spec_file:-}"
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"

# Custom parameters
research_path="${research_path:-}"
subject="${subject:-}"
style="${style:-oil painting}"

# Read research data
if [ -z "$research_path" ] || [ ! -f "$research_path" ]; then
    echo "ERROR: research_path not set or file not found: ${research_path}" >&2
    exit 1
fi

RESEARCH=$(cat "$research_path")

# Craft SDXL prompts from research
claude -p "You are an expert Stable Diffusion prompt engineer specializing in SDXL 1.0.

Given this historical research data:
${RESEARCH}

Craft two prompts optimized for SDXL txt2img generation:

1. A POSITIVE prompt — rich with visual descriptors, art style keywords, quality
   boosters. Use comma-separated keyword style, not sentences. Include the art
   style (${style}), composition, lighting, and detail keywords.

2. A NEGATIVE prompt — terms to exclude (deformities, anachronisms, low quality
   artifacts). Keep it focused and relevant.

Output ONLY valid JSON:
{
  \"subject\": \"${subject}\",
  \"positive\": \"...\",
  \"negative\": \"...\",
  \"width\": 1024,
  \"height\": 1024,
  \"cfg_scale\": 7.0,
  \"steps\": 30,
  \"sampler\": \"euler_ancestral\"
}

Write the JSON to ${message_dir}/prompt.json" \
  --allowedTools 'Bash(cat*),Bash(echo*),Write,Read'

# Verify prompt output exists
if [ ! -f "${message_dir}/prompt.json" ]; then
    echo "ERROR: prompt.json was not created" >&2
    exit 1
fi

# Chain to generate
NEXT_SEQ=$((seq + 1))
cat > ".decree/inbox/${chain}-${NEXT_SEQ}.md" <<CHAIN
---
routine: generate
prompt_path: ${message_dir}/prompt.json
subject: ${subject}
---
Generate portrait image from crafted prompts via ComfyUI.
CHAIN

echo "Prompt crafting complete. Chained to generate."
