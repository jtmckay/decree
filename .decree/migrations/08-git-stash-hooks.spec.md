# 08: Git Stash Lifecycle Hooks (Template)

## Overview

Decree ships template lifecycle hook routines that use git to isolate
each routine's changes into named stashes. The `beforeEach` hook saves
a baseline stash reference, and the `afterEach` hook stashes the delta.
All operations are non-destructive — changes are always saved to named
stashes before any state restoration, so nothing is ever lost.

Decree's core has zero git awareness. All git operations live entirely
in these hook routines. The hooks use `DECREE_ATTEMPT`,
`DECREE_MAX_RETRIES`, and `DECREE_ROUTINE_EXIT_CODE` env vars (provided
by decree to all hooks) to implement retry and exhaustion behavior.

These hooks are offered as templates during `decree init`. They are
regular routines and can be customized or replaced.

## How It Works

### beforeEach: Create or Restore Baseline

On the first attempt, save the current working tree state as a named
baseline stash (without modifying the working tree). On the final retry
attempt, save the failed state and restore the baseline for a clean
slate.

```bash
#!/usr/bin/env bash
# Git Baseline
#
# Saves a baseline stash before routine execution.
# On the final retry, stashes failed changes and restores baseline.
# Used with git-stash-changes (afterEach) to isolate each routine's work.
set -euo pipefail

# --- Parameters ---
# message_file  - Path to message.md in the run directory
# message_id    - Full message ID (e.g., D0001-1432-01-add-auth-0)
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
```

### afterEach: Stash the Routine's Changes

After the routine completes (success or failure), save the current
state as a named stash checkpoint without modifying the working tree.
On exhaustion (all retries failed), save the exhausted state and
restore to baseline.

```bash
#!/usr/bin/env bash
# Git Stash Changes
#
# Stashes the routine's changes as a named checkpoint.
# On exhaustion, saves the failed state and restores baseline.
# Each stash is named with the message ID.
set -euo pipefail

# --- Parameters ---
# message_file  - Path to message.md in the run directory
# message_id    - Full message ID (e.g., D0001-1432-01-add-auth-0)
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
```

### Result

After processing, the user has:
- All changes live in the working tree (as if hooks weren't there)
- Named stash entries per routine execution:
  `decree: D0001-1432-01-add-auth-0 attempt N`
- Baseline stash: `decree-baseline: D0001-1432-01-add-auth-0`
- Failed attempt stashes: `decree-failed: D0001-1432-01-add-auth-0 attempt N`
- Exhausted stashes: `decree-exhausted: D0001-1432-01-add-auth-0`
- `git stash list` shows all routine checkpoints
- `git stash show "stash@{N}"` shows what a specific attempt changed
- Nothing is ever lost — every state transition is saved before restoration

## Hook Environment Variables

Decree passes these env vars to all lifecycle hooks:

| Variable | Description |
|---|---|
| `DECREE_ATTEMPT` | Current attempt number (1-indexed) |
| `DECREE_MAX_RETRIES` | Configured max retries from config |
| `DECREE_ROUTINE_EXIT_CODE` | Exit code of the routine (afterEach only, 0 for beforeEach) |
| `DECREE_HOOK` | Hook type: `beforeAll`, `afterAll`, `beforeEach`, `afterEach` |

These variables allow hooks to implement their own retry and recovery
strategies without decree needing any awareness of git or other VCS tools.

## Retry Support

All retry logic is driven entirely by the hooks using the env vars above.
Decree's core retry loop is simple:

1. Run beforeEach
2. Run routine
3. Run afterEach
4. If routine failed and attempts remain, go to 1
5. If all retries exhausted, dead-letter the message

The hooks decide what to do at each phase:

**Attempt 1** (beforeEach): save baseline stash.
**Non-final retry** (beforeEach): do nothing — changes stay in place,
routine re-runs (AI can learn from its mistakes).
**Final retry** (beforeEach): stash failed changes, restore baseline
for a clean slate.
**Success** (afterEach): save checkpoint stash.
**Exhaustion** (afterEach): save exhausted state, restore baseline.

## Init Integration

During `decree init`, after AI backend selection (see migration 01):

1. Check `which git` and `git rev-parse --is-inside-work-tree`
2. **If git is detected**, prompt:
   ```
   Enable git stash hooks for change tracking? [Y/n]
   ```
   Default is Yes. If accepted:
   - Copy `git-baseline.sh` and `git-stash-changes.sh` to `.decree/routines/`
   - Uncomment the `beforeEach` and `afterEach` git stash hook lines in config.yml
3. **If git is not detected**, skip the prompt entirely:
   - Print: `git not found — skipping lifecycle hook setup`
   - Git stash hook lines stay commented out
   - No git hook routines are created

If the user declines, git stash hook lines stay commented out. The
commented-out git stash workflow is always present in config.yml so
users can enable it later by uncommenting.

## Template Files

Templates live in `src/templates/`:
- `git-baseline.sh` — beforeEach hook
- `git-stash-changes.sh` — afterEach hook

Embedded via `include_str!()` at compile time.

## Acceptance Criteria

- [ ] `decree init` checks for git via `which git` and `git rev-parse`
- [ ] `decree init` offers git stash hooks only when git is detected
- [ ] `decree init` skips hook prompt with message when git is not found
- [ ] Commented-out git stash workflow is always present in config.yml
- [ ] `git-baseline.sh` creates a named baseline stash on first attempt
- [ ] `git-baseline.sh` stashes failed changes and restores baseline on final retry
- [ ] `git-stash-changes.sh` creates named checkpoint stash after each attempt
- [ ] `git-stash-changes.sh` restores baseline on exhaustion
- [ ] No destructive git commands are used (no `git reset`, `git clean`, `git checkout .`)
- [ ] All state transitions are saved to named stashes before restoration
- [ ] Failed attempt stashes are named `decree-failed: <message_id> attempt N`
- [ ] Exhausted stashes are named `decree-exhausted: <message_id>`
- [ ] Baseline stashes are named `decree-baseline: <message_id>`
- [ ] Checkpoint stashes are named `decree: <message_id> attempt N`
- [ ] Working tree retains all changes after afterEach runs (success case)
- [ ] Pre-checks verify git is available and we're in a git repo
- [ ] Hooks use `DECREE_ATTEMPT`, `DECREE_MAX_RETRIES`, `DECREE_ROUTINE_EXIT_CODE`
- [ ] Decree core has zero git awareness — all git logic is in hooks
- [ ] Hooks can be disabled by commenting out config values
