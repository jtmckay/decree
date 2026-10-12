#!/usr/bin/env bash
# onentry script of `done`: commit the run's work, titled with the message's first `# `
# heading. For a migration, decree has already added it to processed.md, so the one commit
# holds the work and the ledger line. Nothing changed is nothing to commit. If the commit
# fails, the run ends in `failed`, not `done`.
set -euo pipefail
git add -A
if git diff --cached --quiet; then
  echo "nothing to commit"
  exit 0
fi
title=$(sed -n 's/^# //p' "${DECREE_MESSAGE}" | head -n 1)
git commit -q -m "${title:-${DECREE_MACHINE}: ${DECREE_MESSAGE_ID}}" \
  -m "decree run ${DECREE_MESSAGE_ID} (${DECREE_MACHINE})"
git log -1 --format='committed %h %s'
