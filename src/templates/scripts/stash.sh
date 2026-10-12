#!/usr/bin/env bash
# onentry script of `failed`: put the run's changes in a git stash named after the run, so the
# next run starts from a clean tree, and say how to get them back. Only a run that started
# from a clean tree (`clean_tree` passed) stashes: one that failed because of a person's own
# uncommitted changes leaves them where they are.
set -euo pipefail
[ -f "${DECREE_RUN_DIR}/clean_tree" ] || exit 0
if [ -z "$(git status --porcelain)" ]; then
  echo "nothing to stash"
  exit 0
fi
title=$(sed -n 's/^# //p' "${DECREE_MESSAGE}" | head -n 1)
git stash push --include-untracked --quiet -m "decree ${DECREE_MESSAGE_ID}: ${title:-${DECREE_MACHINE}}"
echo "stashed the run's changes as $(git stash list -n 1 --format=%gd): \`git stash show -p\` shows them;"
echo "to continue the run, \`git stash pop\`, then \`decree process --retry ${DECREE_MESSAGE_ID}\`"
