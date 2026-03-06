#!/usr/bin/env bash
# Market Analysis
#
# First step in the business evaluation chain. Analyzes the total
# addressable market, serviceable segments, trends, dynamics, and
# risks. Writes to $message_dir/01-market-analysis.md, then chains
# to competitive-landscape via outbox.
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

# Pre-check: verify AI tool is available
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then
    command -v claude >/dev/null 2>&1 || { echo "claude not found" >&2; exit 1; }
    exit 0
fi

IDEA=$(cat "$message_file")

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

# Chain to competitive-landscape via outbox
mkdir -p .decree/outbox
cat > ".decree/outbox/01-competitive-landscape.md" <<CHAIN
---
routine: competitive-landscape
work_file: ${message_file}
market_analysis_path: ${message_dir}/01-market-analysis.md
---
Analyze the competitive landscape for this business idea.
CHAIN

echo "Market analysis complete. Chained to competitive-landscape."
