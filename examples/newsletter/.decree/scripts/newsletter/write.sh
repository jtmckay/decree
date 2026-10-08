#!/usr/bin/env bash
# newsletter's write: a local model reads $DECREE_LIB/newsletter/taste.md and the run's
# items.jsonl, and picks and summarises the items worth reading. Writes
# $DECREE_RUN_DIR/issue.md: a title with the date, then the model's picks as markdown links.
# With no new items it says so, without calling the model.
set -euo pipefail
items="${DECREE_RUN_DIR}/items.jsonl"
taste="${DECREE_LIB}/newsletter/taste.md"
issue="${DECREE_RUN_DIR}/issue.md"
today="$(date +%F)"

if [ ! -s "${items}" ]; then
  printf '# Newsletter, %s\n\nNothing new in your feeds since the last issue.\n' "${today}" > "${issue}"
  echo "write: no new items; wrote the nothing-new issue without calling the model"
  exit 0
fi

instructions="You write a personal newsletter. The reader describes their taste below. Pick the items that fit it, and follow the format it asks for. Reply with the markdown list only: one line per item, '- [title](link): what it is and why it matters'. Use only links from the items given.

$(cat "${taste}")"

request="$(jq -n \
  --arg model "${OLLAMA_MODEL}" \
  --arg system "${instructions}" \
  --rawfile items "${items}" \
  '{model: $model, stream: false, messages: [
     {role: "system", content: $system},
     {role: "user", content: ("The new items, one JSON object per line:\n\n" + $items)}
   ]}')"

echo "write: asking ${OLLAMA_MODEL} at ${OLLAMA_URL} about $(wc -l < "${items}") items"
reply="$(curl -sS --fail --max-time 600 -H 'Content-Type: application/json' \
  -d "${request}" "${OLLAMA_URL}/api/chat")"
picks="$(jq -r '.message.content // empty' <<< "${reply}")"
if [ -z "${picks//[[:space:]]/}" ]; then
  echo "write: the model's reply has no content: ${reply}" >&2
  exit 1
fi

printf '# Newsletter, %s\n\n%s\n' "${today}" "${picks}" > "${issue}.tmp"
mv "${issue}.tmp" "${issue}"
echo "write: wrote issue.md"
