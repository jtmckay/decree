#!/usr/bin/env bash
# competitive_landscape's analyze: direct and indirect competitors,
# positioning and differentiation, building on the market analysis.
set -euo pipefail

# The business idea is the message body: everything after the frontmatter.
IDEA=$(awk 'NR == 1 && /^---$/ { fm = 1; next } fm == 1 && /^---$/ { fm = 2; next } fm != 1' "${DECREE_MESSAGE}")
MARKET=""
if [ -f "${DECREE_DATA_MARKET_ANALYSIS_PATH}" ]; then
  MARKET=$(cat "${DECREE_DATA_MARKET_ANALYSIS_PATH}")
fi

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

Write the complete analysis in markdown to ${DECREE_RUN_DIR}/${DECREE_DATA_REPORT}" \
  --allowedTools 'Bash(cat*),Bash(mkdir*),Write,Read'
