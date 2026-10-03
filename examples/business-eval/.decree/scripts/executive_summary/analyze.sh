#!/usr/bin/env bash
# executive_summary's analyze: a scorecard, strengths, risks and a go/no-go
# recommendation synthesized from all prior analyses.
set -euo pipefail

# The business idea is the message body: everything after the frontmatter.
IDEA=$(awk 'NR == 1 && /^---$/ { fm = 1; next } fm == 1 && /^---$/ { fm = 2; next } fm != 1' "${DECREE_MESSAGE}")
PRIOR=""
for f in "${DECREE_DATA_MARKET_ANALYSIS_PATH}" "${DECREE_DATA_COMPETITIVE_LANDSCAPE_PATH}" "${DECREE_DATA_FINANCIAL_MODEL_PATH}"; do
  if [ -n "$f" ] && [ -f "$f" ]; then
    PRIOR="${PRIOR}

--- $(basename "$f") ---
$(cat "$f")"
  fi
done

claude -p "You are a venture capital analyst preparing an investment memo.
Synthesize all prior analyses into an executive summary and recommendation.

Business idea:
${IDEA}

Prior analyses:
${PRIOR}

Produce:
1. **Business Overview** — one-paragraph summary of the opportunity
2. **Scorecard** — rate each dimension 1-10 with brief justification:
   Market Opportunity, Competitive Position, Business Model Viability,
   Financial Attractiveness, Technical Feasibility, Team Requirements, Timing
3. **Key Strengths** — top 3-5 reasons this could succeed
4. **Key Risks** — top 3-5 risks with mitigations
5. **Critical Assumptions** — what must be true for this to work
6. **Recommended Next Steps** — concrete validation actions
7. **Go / No-Go Recommendation** — clear verdict with reasoning

Be direct and opinionated. Reference specific data from the prior analyses.

Write the complete summary in markdown to ${DECREE_RUN_DIR}/${DECREE_DATA_REPORT}" \
  --allowedTools 'Bash(cat*),Bash(mkdir*),Write,Read'
