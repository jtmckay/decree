#!/usr/bin/env bash
# Executive Summary
#
# Final step in the business evaluation chain. Synthesizes all prior
# analyses into a scorecard with strengths, risks, and a go/no-go
# recommendation. Writes to $message_dir/04-executive-summary.md.
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
financial_model_path="${financial_model_path:-}"

# Read inputs
IDEA=$(cat "$work_file")
PRIOR=""
for f in "$market_analysis_path" "$competitive_landscape_path" "$financial_model_path"; do
    if [ -n "$f" ] && [ -f "$f" ]; then
        PRIOR="${PRIOR}

--- $(basename "$f") ---
$(cat "$f")"
    fi
done

# Generate executive summary
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

Write the complete summary in markdown to ${message_dir}/04-executive-summary.md" \
  --allowedTools 'Bash(cat*),Bash(mkdir*),Write,Read'

# Verify output
if [ ! -f "${message_dir}/04-executive-summary.md" ]; then
    echo "ERROR: 04-executive-summary.md was not created" >&2
    exit 1
fi

echo "Executive summary complete. Evaluation finished."
