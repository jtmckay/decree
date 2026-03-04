# 08: Git Stash Lifecycle Hooks (Template)

## Overview

Decree ships template lifecycle hook routines that use git to isolate
each routine's changes into a named stash. The `beforeEach` hook commits
a baseline, and the `afterEach` hook stashes only the delta — the changes
made between the start and completion of the routine. This replaces any
built-in checkpoint system with standard git tooling the user already knows.

These hooks are offered as templates during `decree init`. They are
regular routines and can be customized or replaced.

## How It Works

### beforeEach: Create Baseline

Before each routine runs, commit the current working tree state as a
temporary baseline commit. This marks the "before" snapshot so we can
isolate the routine's changes afterward.

```bash
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
```

### afterEach: Stash the Routine's Changes

After the routine completes, the working tree contains only the changes
the routine made (relative to the baseline commit). Create a stash
capturing just this delta, then undo the temporary commit.

```bash
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
```

### Result

After processing, the user has:
- All changes live in the working tree (as if hooks weren't there)
- A stash entry per routine execution named `decree: <message_id>`
- `git stash list` shows all routine checkpoints
- `git stash show "stash@{N}"` shows what a specific routine changed
- `git stash pop "stash@{N}"` can restore a specific routine's changes
  after reverting

## Retry Support

The baseline commit enables clean reverts for the retry strategy:

**Non-final retry**: changes stay in place, routine re-runs (AI learns).
No git action needed.

**Final retry**: revert to baseline, re-run with clean slate:
```bash
git checkout . && git clean -fd
```

**Exhaustion**: same revert, then undo baseline:
```bash
git checkout . && git clean -fd
git reset --soft HEAD~1
git reset HEAD .
```

Decree's retry logic checks whether the git stash hooks are active
(config `hooks.beforeEach` is set) and uses git revert instead of its
own checkpoint system when they are.

## Init Integration

During `decree init`, after AI backend selection (see migration 01):

1. Check `which git` and `git rev-parse --is-inside-work-tree`
2. **If git is detected**, prompt:
   ```
   Enable git stash hooks for change tracking? [Y/n]
   ```
   Default is Yes. If accepted:
   - Copy `git-baseline.sh` and `git-stash-changes.sh` to `.decree/routines/`
   - Set `hooks.beforeEach: "git-baseline"` in config.yml
   - Set `hooks.afterEach: "git-stash-changes"` in config.yml
3. **If git is not detected**, skip the prompt entirely:
   - Print: `git not found — skipping lifecycle hook setup`
   - All hook values remain empty strings
   - No git hook routines are created

If the user declines, hooks remain empty (no change tracking).
Hook fields are always present in config.yml regardless.

## Template Files

Templates live in `src/templates/`:
- `git-baseline.sh` — beforeEach hook
- `git-stash-changes.sh` — afterEach hook

Embedded via `include_str!()` at compile time.

## Acceptance Criteria

- [ ] `decree init` checks for git via `which git` and `git rev-parse`
- [ ] `decree init` offers git stash hooks only when git is detected
- [ ] `decree init` skips hook prompt with message when git is not found
- [ ] Hook config fields are always present (empty or populated)
- [ ] `git-baseline.sh` creates a temporary commit before each routine
- [ ] `git-stash-changes.sh` stashes only the routine's delta
- [ ] Stash entries are named `decree: <message_id>`
- [ ] Working tree retains all changes after afterEach runs
- [ ] Baseline commit is removed after afterEach runs
- [ ] Pre-checks verify git is available and we're in a git repo
- [ ] `git stash list` shows one entry per processed routine
- [ ] `git stash show` on an entry shows only that routine's changes
- [ ] Final retry reverts to baseline when git hooks are active
- [ ] Exhaustion reverts and removes baseline commit
- [ ] Hooks can be disabled by clearing config values
