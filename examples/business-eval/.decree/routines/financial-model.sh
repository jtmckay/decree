#!/usr/bin/env bash
# Financial Model
#
# Third step in the business evaluation chain. Builds revenue projections,
# unit economics, cost structure, and funding requirements. Reads prior
# market and competitive analyses. Writes to $message_dir/03-financial-model.md,
# then chains to executive-summary via outbox.
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

# Custom parameters
work_file="${work_file:-}"
market_analysis_path="${market_analysis_path:-}"
competitive_landscape_path="${competitive_landscape_path:-}"
projection_years="${projection_years:-5}"

# Read inputs
IDEA=$(cat "$work_file")
PRIOR=""
for f in "$market_analysis_path" "$competitive_landscape_path"; do
    if [ -n "$f" ] && [ -f "$f" ]; then
        PRIOR="${PRIOR}

--- $(basename "$f") ---
$(cat "$f")"
    fi
done

# Run financial modeling
claude -p "You are a financial analyst specializing in startup modeling.
Build a ${projection_years}-year financial model for the following business idea.

Business idea:
${IDEA}

Prior analyses:
${PRIOR}

Cover:
1. **Revenue Projections** — ${projection_years}-year forecast by stream, with assumptions
2. **Unit Economics** — CAC, LTV, LTV:CAC ratio, payback period, gross margin
3. **Cost Structure** — COGS, operating expenses, fixed vs variable
4. **P&L Summary** — annual revenue, gross profit, EBITDA, net income
5. **Cash Flow** — monthly burn rate by phase, runway calculations
6. **Funding Requirements** — total capital needed, raise schedule, use of funds
7. **Scenario Analysis** — bull/base/bear cases

Use specific dollar amounts, percentages, and unit counts.

Write the complete model in markdown to ${message_dir}/03-financial-model.md" \
  --allowedTools 'Bash(cat*),Bash(mkdir*),Write,Read'

# Verify output
if [ ! -f "${message_dir}/03-financial-model.md" ]; then
    echo "ERROR: 03-financial-model.md was not created" >&2
    exit 1
fi

# Chain to executive-summary via outbox
mkdir -p .decree/outbox
cat > ".decree/outbox/01-executive-summary.md" <<CHAIN
---
routine: executive-summary
work_file: ${work_file}
market_analysis_path: ${market_analysis_path}
competitive_landscape_path: ${competitive_landscape_path}
financial_model_path: ${message_dir}/03-financial-model.md
---
Synthesize all analyses into an executive summary with go/no-go recommendation.
CHAIN

echo "Financial model complete. Chained to executive-summary."
