---
routine: rust-develop
---

# 10: Change Review — Diff and Apply

## Overview

`decree diff` previews what a message's execution changed. `decree apply`
re-applies those changes. They operate on message directories under
`.decree/runs/<chain>-<seq>/`, each containing a `changes.diff` and
`manifest.json` from the checkpoint system (spec 04).

Message IDs use the `<chain>-<seq>` format (defined in spec 01). Commands
accept either a full ID (`2025022514320000-2`) to target a single message,
or just the chain base (`2025022514320000`) to target the entire chain.

## Requirements

### `decree diff` — Preview Changes

```
decree diff                                # latest message
decree diff <chain>-<seq>                  # specific message
decree diff <chain>                        # all messages in a chain
decree diff --since <chain>-<seq>          # all messages from this one onward
```

Behaviour:

- With no arguments: find the most recent message directory, print its
  `changes.diff` to stdout.
- With `<chain>-<seq>`: print that single message's `changes.diff`.
- With `<chain>` (base ID only): concatenate `changes.diff` from all
  messages in the chain (`-0`, `-1`, `-2`, ...) in sequence order.
- With `--since <id>`: concatenate from the specified message through
  the most recent, in chronological order.
- Output is standard unified diff, pipeable to `less`, `delta`, `bat`, etc.
- If the message has no `changes.diff`, print a message and exit cleanly.
- IDs can be specified by any unique prefix.

### `decree apply` — Apply Changes

```
decree apply <chain>-<seq>                 # apply one message's changes
decree apply <chain>                       # apply entire chain in order
decree apply --through <chain>-<seq>       # apply all messages up through this one
decree apply --since <chain>-<seq>         # apply all messages from this one onward
decree apply --all                         # apply all messages in order
```

#### Single Message (`decree apply <chain>-<seq>`)

Apply that one message's `changes.diff`. Cherry-pick — the user takes
responsibility for ensuring the changes apply cleanly.

#### Entire Chain (`decree apply <chain>`)

Apply all messages in the chain (`-0`, `-1`, `-2`, ...) in sequence order.
This reproduces the full chain's work, including all follow-up tasks.

#### Through (`decree apply --through <chain>-<seq>`)

Apply all messages from the oldest in `.decree/runs/` up through and
including the specified message, in chronological order.

#### Since (`decree apply --since <chain>-<seq>`)

Apply all messages from the specified one through the most recent.

#### All (`decree apply --all`)

Apply every message's changes in chronological order.

### Pre-Apply Checks

Before applying any diff, decree performs a dry run:

1. **Parse the diff**: read all hunks and identify affected files.
2. **Check for conflicts**: for each modified file, verify the pre-image
   lines in the diff match the current file content. For new files, verify
   the target path doesn't already exist. For deleted files, verify the
   file exists and matches the expected content.
3. **Report conflicts**: if any hunks can't apply cleanly, print a summary
   of conflicting files and abort. Do not partially apply.

If all hunks apply cleanly, proceed with the actual application.

### Conflict Reporting

When conflicts are detected, print a clear report:

```
Cannot apply changes from 2025022514320000-0 -- conflicts detected:

  src/main.rs:
    hunk at line 42: expected "fn old_code()" but found "fn different_code()"
  src/new_file.rs:
    file already exists (would be overwritten)

Aborting. No files were modified.
Hint: use `decree diff 2025022514320000-0` to inspect the changes.
```

### Force Apply

```
decree apply <chain>-<seq> --force
```

When `--force` is passed, skip the conflict check and apply changes
unconditionally:
- For modified files: overwrite with the post-image content from the diff
- For new files: create/overwrite
- For deleted files: delete

This is destructive — warn the user before proceeding:
```
WARNING: --force will overwrite conflicting files. Continue? [y/N]
```

### Listing Messages

```
decree apply                               # no args: list available messages
```

When called with no arguments, list all message directories grouped by
chain:

```
Messages:

  2025022514320000  spec  01-add-auth.spec.md
    -0  +142 -23 (5 files)
    -1  +12  -8  (2 files)  task: fix type errors

  2025022514321500  spec  02-add-tests.spec.md
    -0  +89  -0  (3 files)

  2025022514324500  spec  03-add-logging.spec.md
    -0  +34  -12 (2 files)
    -1  +5   -2  (1 file)   task: update log format
    -2  +3   -1  (1 file)   task: fix import
```

Chains are listed chronologically. Within each chain, messages are listed
by sequence number. Diff stats and task descriptions provide context.

### Apply Confirmation

Before applying (in non-force mode), show a summary and confirm:

```
Will apply chain 2025022514320000 (2 messages):

  -0  01-add-auth    +142 -23  (5 files)
  -1  fix types      +12  -8   (2 files)

  Total: +154 -31 (7 files)

Proceed? [Y/n]
```

### Integration with Execution Commands

`decree run` and `decree process` leave changes live in the working tree
after successful execution — `decree apply` is for re-applying changes
from previous messages to a different state of the tree (e.g. after a
manual revert, on a fresh clone, or after reverting).

Typical workflow:
1. `decree process` — processes specs, changes are live, checkpoints saved
2. User reviews, decides to revert manually
3. `decree diff 2025022514320000` — review an entire chain's changes
4. `decree diff 2025022514320000-0` — review just the root message
5. `decree apply 2025022514320000` — re-apply the full chain
6. `decree apply 2025022514321500-0` — cherry-pick just the root of another chain

### Error Handling

- **Message not found**: print available messages and exit.
- **No changes.diff**: print message explaining the message had no changes
  or is still in progress.
- **Partially applied (interrupted)**: if the process is killed mid-apply,
  some files may be partially modified. The user can re-run `decree apply`
  (the dry-run check will detect mismatches) or use `decree diff` to
  manually inspect.

### No Git Dependency

Both commands work identically whether or not the project is a git
repository. All diff parsing and application is pure Rust.

## Acceptance Criteria

- **Given** a completed message with a checkpoint diff
  **When** the user runs `decree diff`
  **Then** the changes from the most recent message are printed in unified
  diff format

- **Given** a full message ID `<chain>-<seq>` is provided
  **When** the user runs `decree diff <chain>-<seq>`
  **Then** only that single message's `changes.diff` is printed

- **Given** a chain base ID is provided
  **When** the user runs `decree diff <chain>`
  **Then** all messages in the chain are concatenated in sequence order

- **Given** a full message ID is provided
  **When** the user runs `decree apply <chain>-<seq>`
  **Then** that single message's changes are applied after confirmation

- **Given** a chain base ID is provided
  **When** the user runs `decree apply <chain>`
  **Then** all messages in the chain are applied in sequence order

- **Given** the `--through` flag is used
  **When** the user runs `decree apply --through <chain>-<seq>`
  **Then** all messages from the oldest through the specified one are
  applied in order

- **Given** a diff cannot be applied cleanly
  **When** the dry-run check detects conflicts
  **Then** a conflict report is printed and no files are modified

- **Given** the `--force` flag is used
  **When** conflicts exist
  **Then** the user is warned and changes are applied unconditionally on
  confirmation

- **Given** no arguments are passed to `decree apply`
  **When** the command runs
  **Then** messages are listed grouped by chain, with sequence numbers,
  diff stats, and descriptions

- **Given** `decree diff` output is piped
  **When** the output is consumed by another tool (less, delta, bat)
  **Then** the unified diff format is compatible and renders correctly

- **Given** the project is not a git repository
  **When** `decree apply` or `decree diff` is used
  **Then** both commands work identically — no git dependency
