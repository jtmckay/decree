#!/usr/bin/env bash
# Git Stash Changes
#
# Stashes only the changes made by the routine, then undoes the
# temporary baseline commit. Each stash is named with the message ID.
set -euo pipefail

# --- Parameters ---
# message_file  - Path to message.md in the run directory
# message_id    - Full message ID (<chain>-<seq>)
# message_dir   - Run directory path
# chain         - Chain ID
# seq           - Sequence number
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"

if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then
    command -v git >/dev/null 2>&1 || { echo "git not found" >&2; exit 1; }
    exit 0
fi

# Stage the routine's changes (these are the only changes since baseline)
git add -A

# Create a stash entry without removing changes from working tree
STASH_REF=$(git stash create)

if [ -n "$STASH_REF" ]; then
    git stash store -m "decree: ${message_id}" "$STASH_REF"
    echo "Stashed routine changes: decree: ${message_id}"
fi

# Undo the temporary baseline commit, keeping all changes in working tree
git reset --soft HEAD~1
git reset HEAD .
