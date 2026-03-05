#!/usr/bin/env bash
# Git Stash Changes
#
# Stashes the routine's changes as a named checkpoint.
# On exhaustion, saves the failed state and restores baseline.
# Each stash is named with the message ID.
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
    exit 0
fi

ATTEMPT="${DECREE_ATTEMPT:-1}"
MAX_RETRIES="${DECREE_MAX_RETRIES:-3}"
EXIT_CODE="${DECREE_ROUTINE_EXIT_CODE:-0}"

# Always save current state as a named checkpoint (non-destructive)
git add -A
STASH_REF=$(git stash create)
if [ -n "$STASH_REF" ]; then
    git stash store -m "decree: ${message_id} attempt ${ATTEMPT}" "$STASH_REF"
    echo "Stashed routine changes: decree: ${message_id} attempt ${ATTEMPT}"
fi

# On exhaustion: save failed state, restore baseline
if [ "$EXIT_CODE" -ne 0 ] && [ "$ATTEMPT" -eq "$MAX_RETRIES" ]; then
    # Save the exhausted state (so nothing is lost)
    git stash push --include-untracked -m "decree-exhausted: ${message_id}" 2>/dev/null || true

    # Restore baseline
    BASELINE_IDX=$(git stash list | grep -m1 "decree-baseline: ${message_id}" | sed 's/stash@{\([0-9]*\)}.*/\1/')
    if [ -n "$BASELINE_IDX" ]; then
        git stash apply "stash@{$BASELINE_IDX}" 2>/dev/null || true
        echo "Exhausted — restored to baseline"
    fi
fi
