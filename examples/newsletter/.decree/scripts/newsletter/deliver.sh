#!/usr/bin/env bash
# newsletter's deliver: copy the run's issue.md to $NEWSLETTER_DIR/<YYYY-MM-DD>.md (-2, -3
# when that day already has an issue), record every gathered link in $DECREE_STORE/seen.tsv
# with the date, and, if NTFY_URL is set, post the issue's first three lines to
# $NTFY_URL/$NTFY_TOPIC. Safe to re-run: an identical issue is not copied twice, and a link
# already in seen.tsv is not added again.
set -euo pipefail
dir="${NEWSLETTER_DIR:-newsletter}"
issue="${DECREE_RUN_DIR}/issue.md"
items="${DECREE_RUN_DIR}/items.jsonl"
today="$(date +%F)"
mkdir -p "${dir}"

# The day's file name: the first free one, or the one already holding this issue.
target="${dir}/${today}.md"
n=1
while [ -e "${target}" ] && ! cmp -s "${issue}" "${target}"; do
  n=$((n + 1))
  target="${dir}/${today}-${n}.md"
done
cp "${issue}" "${target}"
echo "deliver: ${target}"

seen="${DECREE_STORE}/seen.tsv"
touch "${seen}"
added=0
if [ -s "${items}" ]; then
  while IFS= read -r link; do
    if ! cut -f1 "${seen}" | grep -qxF -- "${link}"; then
      printf '%s\t%s\n' "${link}" "${today}" >> "${seen}"
      added=$((added + 1))
    fi
  done < <(jq -r '.link' "${items}")
fi
echo "deliver: ${added} links added to ${seen}"

if [ -z "${NTFY_URL:-}" ]; then
  echo "deliver: NTFY_URL is not set; skipping the ntfy ping"
  exit 0
fi
if [ -z "${NTFY_TOPIC:-}" ]; then
  echo "deliver: NTFY_URL is set but NTFY_TOPIC is not" >&2
  exit 1
fi
head -n 3 "${target}" | curl -sS --fail --max-time 30 -H "Title: $(basename "${target}")" \
  --data-binary @- "${NTFY_URL%/}/${NTFY_TOPIC}" > /dev/null
echo "deliver: pinged ${NTFY_URL%/}/${NTFY_TOPIC}"
