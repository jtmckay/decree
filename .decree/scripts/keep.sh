#!/usr/bin/env bash
# onentry script of `failed`: leave the run's changes in the work tree, so a retry builds on
# the last attempt (the agent sees them in `git status` and decides what to keep), and hold
# the queue with .decree/HOLD, so no other run starts on them. `decree process --retry` of
# this run removes HOLD; to set the changes aside instead, stash them and delete HOLD.
# Only the project folder counts (`-- .`): a decree project can be a subfolder of a larger
# repository. A run that failed because a person's own changes were there from the start
# (`clean_tree` did not pass) leaves them alone and holds nothing.
set -euo pipefail
[ -f "${DECREE_RUN_DIR}/clean_tree" ] || exit 0
changes=$(git status --porcelain -- .)
if [ -z "${changes}" ]; then
  echo "no changes to keep"
  exit 0
fi
title=$(sed -n 's/^# //p' "${DECREE_MESSAGE}" | head -n 1)
{
  echo "${DECREE_MESSAGE_ID}"
  echo "run ${DECREE_MESSAGE_ID} (${title:-${DECREE_MACHINE}}) failed and kept its changes in the work tree:"
  echo "${changes}"
  echo "to continue the run on them: \`decree process --retry ${DECREE_MESSAGE_ID}\`"
  echo "to set them aside instead: \`git stash push --include-untracked -m \"decree ${DECREE_MESSAGE_ID}\" -- .\`, then delete .decree/HOLD"
} > "${DECREE_PROJECT_ROOT}/.decree/HOLD"
tail -n +2 "${DECREE_PROJECT_ROOT}/.decree/HOLD"
echo "no other run starts until then (.decree/HOLD)"
