#!/usr/bin/env bash
# local_router's only script. Sends the request's question, options (with
# their descriptions) and input to a local GLiNER2.5-Decide server, and writes
# its top label as the reply, with the per-option scores. decree applies
# min_confidence; this script only reports.
#
# The server is yours: a few lines of Python around gliner2's classify_text
# with include_confidence=True, loaded once (docs/routers.md, docs/services.md).
set -euo pipefail
url="${DECIDE_URL:-http://127.0.0.1:8090/classify}"
body=$(jq '{instructions: .question,
            labels: (.options | map({(.event): .description}) | add),
            text: .input}' "$DECREE_REQUEST")
echo "$body"
echo "--- reply"
scores=$(curl -fsS --max-time 30 -H 'content-type: application/json' -d "$body" "$url")
echo "$scores"
# {"invoice": 0.31, "receipt": 0.62, "other": 0.07} -> the top label and its score
jq '{probabilities: .} + (to_entries | max_by(.value) | {event: .key, confidence: .value})' \
  <<<"$scores" > "$DECREE_REPLY"
# a plain last line, as in router/ask_claude.sh
jq -r '"picked \(.event)"' "$DECREE_REPLY"
