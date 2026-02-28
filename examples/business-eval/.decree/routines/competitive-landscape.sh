#!/usr/bin/env bash
# Competitive Landscape
#
# Second step in the business evaluation chain. Maps direct and indirect
# competitors, analyzes positioning, and identifies differentiation.
# Reads prior market analysis. Writes to $message_dir/02-competitive-landscape.md,
# then chains to financial-model.
set -euo pipefail

# Parameters (decree injects these as env vars)
spec_file="${spec_file:-}"
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"

# Custom parameters
work_file="${work_file:-}"
market_analysis_path="${market_analysis_path:-}"

# Read inputs
IDEA=$(cat "$work_file")
MARKET=""
if [ -n "$market_analysis_path" ] && [ -f "$market_analysis_path" ]; then
    MARKET=$(cat "$market_analysis_path")
fi

# Run competitive analysis
claude -p "You are a competitive intelligence analyst. Analyze the competitive
landscape for the following business idea.

Business idea:
${IDEA}

Prior market analysis:
${MARKET}

Cover:
1. **Direct Competitors** — similar products/services with pricing, strengths, weaknesses
2. **Indirect Competitors** — adjacent solutions addressing the same need
3. **Competitive Matrix** — feature comparison table across key dimensions
4. **Positioning Map** — where each player sits on price vs. feature axes
5. **Differentiation Strategy** — what makes this idea defensible
6. **Competitive Threats** — incumbent responses, new entrant risk
7. **Strategic Moats** — network effects, data advantages, switching costs

Reference specific competitor products and pricing by name.

Write the complete analysis in markdown to ${message_dir}/02-competitive-landscape.md" \
  --allowedTools 'Bash(cat*),Bash(mkdir*),Write,Read'

# Verify output
if [ ! -f "${message_dir}/02-competitive-landscape.md" ]; then
    echo "ERROR: 02-competitive-landscape.md was not created" >&2
    exit 1
fi

# Chain to financial-model
NEXT_SEQ=$((seq + 1))
cat > ".decree/inbox/${chain}-${NEXT_SEQ}.md" <<CHAIN
---
routine: financial-model
work_file: ${work_file}
market_analysis_path: ${market_analysis_path}
competitive_landscape_path: ${message_dir}/02-competitive-landscape.md
---
Build a financial model for this business idea.
CHAIN

echo "Competitive landscape complete. Chained to financial-model."
