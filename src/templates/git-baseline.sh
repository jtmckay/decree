#!/usr/bin/env bash
# Git Baseline
#
# Creates a temporary git commit as a baseline before routine execution.
# Used with git-stash-changes (afterEach) to isolate each routine's work.
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
    git rev-parse --is-inside-work-tree >/dev/null 2>&1 || { echo "not a git repo" >&2; exit 1; }
    exit 0
fi

# Stage everything and create a temporary baseline commit
git add -A
git commit --allow-empty --no-verify -m "decree-baseline: ${message_id}"
