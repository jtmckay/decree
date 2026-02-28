#!/usr/bin/env bash
# Research
#
# First step in the portrait chain. Uses AI to research the historical
# figure's appearance at a specific moment in time. Writes structured
# research data to $message_dir/research.json, then chains to prompt-craft.
set -euo pipefail

# Parameters (decree injects these as env vars)
spec_file="${spec_file:-}"
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"

# Custom parameters
subject="${subject:-}"
moment="${moment:-}"
style="${style:-oil painting}"

# Determine the work description
if [ -n "$spec_file" ] && [ -f "$spec_file" ]; then
    WORK_FILE="$spec_file"
else
    WORK_FILE="$message_file"
fi

# Research the historical figure's appearance
claude -p "You are a historical research assistant. Research the physical appearance of
${subject} at the moment of ${moment}.

Include:
- Physical description (age, build, hair, distinguishing features)
- Clothing and accessories appropriate to the moment
- Setting and environmental details
- Lighting conditions and atmosphere
- Art style notes for: ${style}

Read the full context from ${WORK_FILE} for additional details.

Output ONLY valid JSON in this format:
{
  \"subject\": \"...\",
  \"moment\": \"...\",
  \"style\": \"...\",
  \"physical\": { \"age\": \"...\", \"build\": \"...\", \"hair\": \"...\", \"features\": \"...\" },
  \"clothing\": \"...\",
  \"setting\": \"...\",
  \"lighting\": \"...\",
  \"atmosphere\": \"...\",
  \"art_notes\": \"...\"
}

Write the JSON to ${message_dir}/research.json" \
  --allowedTools 'Bash(cat*),Bash(echo*),Write,Read'

# Verify research output exists
if [ ! -f "${message_dir}/research.json" ]; then
    echo "ERROR: research.json was not created" >&2
    exit 1
fi

# Chain to prompt-craft
NEXT_SEQ=$((seq + 1))
cat > ".decree/inbox/${chain}-${NEXT_SEQ}.md" <<CHAIN
---
routine: prompt-craft
research_path: ${message_dir}/research.json
subject: ${subject}
style: ${style}
---
Craft SDXL image generation prompts from research data.
CHAIN

echo "Research complete. Chained to prompt-craft."
