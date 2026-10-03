#!/usr/bin/env bash
# market_analysis's analyze: TAM/SAM/SOM, trends, segments and risks for the
# idea in the message body, written to the run folder as the machine's report.
set -euo pipefail

# The business idea is the message body: everything after the frontmatter.
IDEA=$(awk 'NR == 1 && /^---$/ { fm = 1; next } fm == 1 && /^---$/ { fm = 2; next } fm != 1' "${DECREE_MESSAGE}")

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

Write the complete analysis in markdown to ${DECREE_RUN_DIR}/${DECREE_DATA_REPORT}" \
  --allowedTools 'Bash(cat*),Bash(mkdir*),Write,Read'
