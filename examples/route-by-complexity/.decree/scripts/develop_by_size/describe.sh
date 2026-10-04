#!/usr/bin/env bash
# develop_by_size's describe: prints what the classifier reads, which is the
# message's title, its acceptance criteria, and the files it names that exist
# here with their line counts, so the classifier sees a size, not just prose.
set -euo pipefail
msg="$DECREE_MESSAGE"
grep -m1 '^# ' "$msg" || true
echo
# the Acceptance Criteria section, up to the next ## heading
awk '/^## / { on = ($0 ~ /^## Acceptance Criteria/) } on' "$msg"
echo
echo "Files it names (lines, path):"
files=0
lines=0
while read -r path; do
  if [ -f "$path" ]; then
    n=$(wc -l < "$path")
    printf '%6d %s\n' "$n" "$path"
    files=$((files + 1))
    lines=$((lines + n))
  fi
done < <(grep -oE '[A-Za-z0-9_./-]+\.[A-Za-z0-9]+' "$msg" | sort -u || true)
echo "Total: $lines lines in $files files"
