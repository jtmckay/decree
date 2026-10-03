#!/usr/bin/env bash
# Post runs/<id>/comfy-payload.json (written by the machine's build_payload)
# to the ComfyUI API and save the reply as runs/<id>/comfy-response.json.
set -euo pipefail
payload="${DECREE_RUN_DIR}/comfy-payload.json"

response=$(curl -s -w "\n%{http_code}" -X POST "${DECREE_DATA_API_URL}" \
  -H 'Content-Type: application/json' \
  --data-binary "@${payload}")

http_code=$(echo "$response" | tail -1)
body=$(echo "$response" | sed '$d')

echo "=== Response (HTTP $http_code) ==="
echo "$body"
echo "$body" > "${DECREE_RUN_DIR}/comfy-response.json"

[ "$http_code" -ge 200 ] && [ "$http_code" -lt 300 ]
