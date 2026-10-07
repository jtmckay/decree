#!/usr/bin/env bash
# Print the scan's text. Later checks and models read this output.
set -euo pipefail
pdftotext -layout "$DECREE_DATA_FILE" -
