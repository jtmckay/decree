#!/usr/bin/env bash
# Market Analysis
#
# First step in the business evaluation chain. Analyzes the total
# addressable market, serviceable segments, trends, dynamics, and
# risks. Writes to $message_dir/01-market-analysis.md, then chains
# to competitive-landscape.
set -euo pipefail

# Parameters (decree injects these as env vars)
spec_file="${spec_file:-}"
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"

# Determine the work description
if [ -n "$spec_file" ] && [ -f "$spec_file" ]; then
    WORK_FILE="$spec_file"
else
    WORK_FILE="$message_file"
fi

IDEA=$(cat "$WORK_FILE")

# Run market analysis
claude -p "You are a market research analyst. Analyze the following business idea
and produce a comprehensive market analysis.

Business idea:
${IDEA}

Cover:
1. **Total Addressable Market (TAM)** — global market size with reasoning
2. **Serviceable Addressable Market (SAM)** — realistic reachable market
3. **Serviceable Obtainable Market (SOM)** — achievable share in years 1-3
4. **Market Trends** — growth drivers, technology shifts, regulatory changes
5. **Customer Segments** — primary and secondary segments with personas
6. **Market Dynamics** — supply/demand, pricing trends, distribution channels
7. **Risks & Barriers** — market risks, adoption barriers, timing risks

Use specific numbers and percentages where possible.

Write the complete analysis in markdown to ${message_dir}/01-market-analysis.md" \
  --allowedTools 'Bash(cat*),Bash(mkdir*),Write,Read'

# Verify output
if [ ! -f "${message_dir}/01-market-analysis.md" ]; then
    echo "ERROR: 01-market-analysis.md was not created" >&2
    exit 1
fi

# Chain to competitive-landscape
NEXT_SEQ=$((seq + 1))
cat > ".decree/inbox/${chain}-${NEXT_SEQ}.md" <<CHAIN
---
routine: competitive-landscape
work_file: ${WORK_FILE}
market_analysis_path: ${message_dir}/01-market-analysis.md
---
Analyze the competitive landscape for this business idea.
CHAIN

echo "Market analysis complete. Chained to competitive-landscape."
