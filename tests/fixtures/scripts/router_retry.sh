#!/usr/bin/env bash
# A router script: copies the request to the run folder and replies retry with confidence 0.9.
set -euo pipefail
cp "$DECREE_REQUEST" "$DECREE_RUN_DIR/request_copy.json"
echo '{"event":"retry","confidence":0.9}' > "$DECREE_REPLY"
