#!/usr/bin/env bash
# Fail unless analyze wrote the machine's report into the run folder.
set -euo pipefail
report="${DECREE_RUN_DIR}/${DECREE_DATA_REPORT}"
if [ ! -s "$report" ]; then
  echo "ERROR: ${DECREE_DATA_REPORT} was not created" >&2
  exit 1
fi
echo "Report: ${report}"
