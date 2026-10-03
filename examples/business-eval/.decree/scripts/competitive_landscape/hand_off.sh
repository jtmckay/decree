#!/usr/bin/env bash
# competitive_landscape's hand_off: queue financial_model with the same idea
# and the reports so far.
set -euo pipefail
awk 'NR == 1 && /^---$/ { fm = 1; next } fm == 1 && /^---$/ { fm = 2; next } fm != 1' "${DECREE_MESSAGE}" \
  | decree emit --machine financial_model \
      --param "market_analysis_path=${DECREE_DATA_MARKET_ANALYSIS_PATH}" \
      --param "competitive_landscape_path=${DECREE_RUN_DIR}/${DECREE_DATA_REPORT}"
echo "Competitive landscape complete. Queued financial_model."
