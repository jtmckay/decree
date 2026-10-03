#!/usr/bin/env bash
# market_analysis's hand_off: queue competitive_landscape with the same idea
# as its body and this run's report as a param.
set -euo pipefail
awk 'NR == 1 && /^---$/ { fm = 1; next } fm == 1 && /^---$/ { fm = 2; next } fm != 1' "${DECREE_MESSAGE}" \
  | decree emit --machine competitive_landscape \
      --param "market_analysis_path=${DECREE_RUN_DIR}/${DECREE_DATA_REPORT}"
echo "Market analysis complete. Queued competitive_landscape."
