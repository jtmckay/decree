#!/usr/bin/env bash
# gliner_router's only script: one classification per decision. Sends the
# request's question, its options with their descriptions, and its input and
# message body to the GLiNER2.5-Decide server, and writes the server's answer,
# {"event": ..., "confidence": ...}, as the reply. decree applies
# min_confidence; this script only reports. If the server is down, curl fails,
# the router run ends in `failed`, and the deciding state's event is `error`.
# The server: examples/route-by-complexity/gliner/decide_server.py.
set -euo pipefail
url="${GLINER_URL:-http://127.0.0.1:8090/classify}"
body=$(jq '{instructions: .question,
            labels: (.options | map({(.event): .description}) | add),
            text: ([.input, .message_body] | map(select(. != "")) | join("\n\n"))}' "$DECREE_REQUEST")
echo "$body"
echo "--- reply"
reply=$(curl -fsS --max-time 30 -H 'content-type: application/json' -d "$body" "$url")
echo "$reply"
printf '%s\n' "$reply" > "$DECREE_REPLY"
# for the log only: decree takes the event from the reply
jq -r '"picked \(.event)"' "$DECREE_REPLY"
