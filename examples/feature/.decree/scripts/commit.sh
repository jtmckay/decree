#!/usr/bin/env bash
# onentry script of done. For a migration, decree has already added it to
# processed.md, so this one commit carries the code and the ledger line.
# If this fails, decree removes the ledger line and the run ends in failed.
set -euo pipefail
title=$(grep -m1 '^# ' "$DECREE_MESSAGE" | sed 's/^# //')
git add -A
git commit -q -m "$title" -m "decree run $DECREE_MESSAGE_ID"
git log -1 --format='committed %h %s'
