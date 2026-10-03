#!/usr/bin/env bash
# financial_model's hand_off: queue executive_summary with the same idea and
# all three reports.
set -euo pipefail
awk 'NR == 1 && /^---$/ { fm = 1; next } fm == 1 && /^---$/ { fm = 2; next } fm != 1' "${DECREE_MESSAGE}" \
  | decree emit --machine executive_summary \
      --param "market_analysis_path=${DECREE_DATA_MARKET_ANALYSIS_PATH}" \
      --param "competitive_landscape_path=${DECREE_DATA_COMPETITIVE_LANDSCAPE_PATH}" \
      --param "financial_model_path=${DECREE_RUN_DIR}/${DECREE_DATA_REPORT}"
echo "Financial model complete. Queued executive_summary."
