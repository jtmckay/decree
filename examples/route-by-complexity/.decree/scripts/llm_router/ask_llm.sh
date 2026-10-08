#!/usr/bin/env bash
# llm_router's only script: one chat call per decision. Posts the request's
# question, options, input and message body to Ollama's /api/chat with
# `format` set to the request's reply_schema, so the decoder can only write a
# reply that schema allows: `reason` first, then an `event` that is one of the
# options. decree applies min_confidence; this script only reports. If Ollama
# is down, curl fails, the router run ends in `failed`, and the deciding
# state's event is `error`.
#
# llama.cpp's server (llama-server) has no /api/chat; use its OpenAI-compatible
# endpoint instead: post to http://127.0.0.1:8080/v1/chat/completions with
#   response_format: {type: "json_schema", json_schema: {schema: .reply_schema}}
# in place of `format`, and read the reply from .choices[0].message.content
# instead of .message.content.
set -euo pipefail
url="${LLM_URL:-http://127.0.0.1:11434/api/chat}"
model="${LLM_MODEL:-qwen3-coder:30b}"
body=$(jq --arg model "$model" '{
  model: $model, stream: false, format: .reply_schema, options: {temperature: 0},
  messages: [{role: "user", content: (
    "Question: \(.question)\n\nOptions:\n" +
    (.options | map("- \(.event): \(.description)") | join("\n")) +
    "\n\nOutput this decision reads:\n\(.input)\n\nTask message:\n\(.message_body)\n\n" +
    "Give your reason first, then the event. " +
    "Reply with JSON that matches this schema:\n" + (.reply_schema | tojson))}]
}' "$DECREE_REQUEST")
echo "$body"
echo "--- reply"
answer=$(curl -fsS --max-time 120 -H 'content-type: application/json' -d "$body" "$url")
echo "$answer"
jq '.message.content | fromjson' <<<"$answer" > "$DECREE_REPLY"
# for the log only: decree takes the event from the reply
jq -r '"picked \(.event): \(.reason // "")"' "$DECREE_REPLY"
