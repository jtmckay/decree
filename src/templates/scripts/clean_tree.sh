#!/usr/bin/env bash
# Root onentry script: the run starts from a clean git work tree, so the commit at `done`
# holds this run's work only, and a failed run's changes never mix into the next one.
# It runs again when `decree process --retry` continues the run; by then the tree holds
# the run's own work, so it checks only the first time. Only the project folder counts
# (`-- .`): a decree project can be a subfolder of a larger repository.
set -euo pipefail
mark="${DECREE_RUN_DIR}/clean_tree"
[ -f "${mark}" ] && exit 0
command -v git >/dev/null || { echo "clean_tree: git not found on PATH" >&2; exit 1; }
git rev-parse --is-inside-work-tree >/dev/null 2>&1 || {
  echo "clean_tree: not a git work tree; run \`git init\` and commit first" >&2
  exit 1
}
changes=$(git status --porcelain -- .)
if [ -n "${changes}" ]; then
  echo "clean_tree: uncommitted changes; commit or stash them, then run the message again:" >&2
  echo "${changes}" | head -n 20 >&2
  n=$(echo "${changes}" | wc -l)
  [ "${n}" -le 20 ] || echo "… and $((n - 20)) more" >&2
  echo "if a failed run left them, \`decree status\` names it; continue it with \`decree process --retry <id>\`" >&2
  exit 1
fi
git rev-parse HEAD > "${mark}"
echo "clean at $(git rev-parse --short HEAD)"
