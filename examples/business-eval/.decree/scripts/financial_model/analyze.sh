#!/usr/bin/env bash
# financial_model's analyze: revenue projections, unit economics, cost
# structure and funding needs, building on the prior analyses.
set -euo pipefail

# The business idea is the message body: everything after the frontmatter.
IDEA=$(awk 'NR == 1 && /^---$/ { fm = 1; next } fm == 1 && /^---$/ { fm = 2; next } fm != 1' "${DECREE_MESSAGE}")
years="${DECREE_DATA_PROJECTION_YEARS}"
PRIOR=""
for f in "${DECREE_DATA_MARKET_ANALYSIS_PATH}" "${DECREE_DATA_COMPETITIVE_LANDSCAPE_PATH}"; do
  if [ -n "$f" ] && [ -f "$f" ]; then
    PRIOR="${PRIOR}

--- $(basename "$f") ---
$(cat "$f")"
  fi
done

claude -p "You are a financial analyst specializing in startup modeling.
Build a ${years}-year financial model for the following business idea.

Business idea:
${IDEA}

Prior analyses:
${PRIOR}

Cover:
1. **Revenue Projections** — ${years}-year forecast by stream, with assumptions
2. **Unit Economics** — CAC, LTV, LTV:CAC ratio, payback period, gross margin
3. **Cost Structure** — COGS, operating expenses, fixed vs variable
4. **P&L Summary** — annual revenue, gross profit, EBITDA, net income
5. **Cash Flow** — monthly burn rate by phase, runway calculations
6. **Funding Requirements** — total capital needed, raise schedule, use of funds
7. **Scenario Analysis** — bull/base/bear cases

Use specific dollar amounts, percentages, and unit counts.

Write the complete model in markdown to ${DECREE_RUN_DIR}/${DECREE_DATA_REPORT}" \
  --allowedTools 'Bash(cat*),Bash(mkdir*),Write,Read'
