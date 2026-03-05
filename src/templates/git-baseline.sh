#!/usr/bin/env bash
# Git Baseline
#
# Saves a baseline stash before routine execution.
# On the final retry, stashes failed changes and restores baseline.
# Used with git-stash-changes (afterEach) to isolate each routine's work.
set -euo pipefail

# --- Standard Environment Variables ---
# message_file  - Path to message.md in the run directory
# message_id    - Full message ID (e.g., D0001-1432-01-add-auth-0)
# message_dir   - Run directory path (contains logs from prior attempts)
# chain         - Chain ID (D<NNNN>-HHmm-<name>)
# seq           - Sequence number in chain
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

ATTEMPT="${DECREE_ATTEMPT:-1}"
MAX_RETRIES="${DECREE_MAX_RETRIES:-3}"

if [ "$ATTEMPT" -eq 1 ]; then
    # First attempt: save current state as named baseline stash
    git add -A
    BASELINE=$(git stash create)
    if [ -n "$BASELINE" ]; then
        git stash store -m "decree-baseline: ${message_id}" "$BASELINE"
        echo "Baseline saved: decree-baseline: ${message_id}"
    else
        echo "Clean working tree, no baseline needed"
    fi
elif [ "$ATTEMPT" -eq "$MAX_RETRIES" ]; then
    # Final retry: save failed state, restore baseline for clean slate
    git stash push --include-untracked -m "decree-failed: ${message_id} attempt $((ATTEMPT - 1))" 2>/dev/null || true

    BASELINE_IDX=$(git stash list | grep -m1 "decree-baseline: ${message_id}" | sed 's/stash@{\([0-9]*\)}.*/\1/')
    if [ -n "$BASELINE_IDX" ]; then
        git stash apply "stash@{$BASELINE_IDX}" 2>/dev/null || true
        echo "Restored baseline for final retry"
    fi
fi
