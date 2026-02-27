---
routine: rust-develop
---

# 04: Self-Contained Checkpoints for Change Tracking and Reversion

## Overview

Decree has its own built-in checkpoint system for tracking and reverting
file changes — no git installation or git repository required. Each
message execution creates a checkpoint: a manifest of the project tree
before execution and a unified diff of what changed. Checkpoints are
stored in the message's directory under `.decree/runs/<msg-id>/`. Revert
applies the diff in reverse.

All logic is implemented in pure Rust using crates for directory walking
(`walkdir`), diffing (`similar` or `diffy`), hashing (`sha2`), and
gitignore parsing (`ignore`). No external tools are shelled out to.

## Requirements

### Checkpoint Location

Each message directory contains its own checkpoint. Message IDs have the
form `<chain>-<seq>` (defined in spec 01):

```
.decree/runs/<chain>-<seq>/
├── message.md               # Copy of the inbox message
├── manifest.json            # Tree state before execution
├── changes.diff             # What this execution changed (unified diff)
├── routine.log              # Execution log (shell script routines)
├── output.ipynb             # Papermill output notebook (notebook routines)
└── papermill.log            # Execution stderr (notebook routines)
```

### Manifest Format

A manifest is a JSON file recording every file in the project tree at a
point in time:

```json
{
  "files": {
    "src/main.rs": {
      "sha256": "e3b0c44298fc1c149afbf4c8996fb924...",
      "size": 1234,
      "mode": "644"
    },
    "src/lib.rs": {
      "sha256": "d7a8fbb307d7809469ca9abcb0082e4f...",
      "size": 5678,
      "mode": "644"
    }
  }
}
```

Fields per file:
- `sha256` — hex-encoded SHA-256 hash of file contents
- `size` — file size in bytes
- `mode` — unix permission bits (stored for accurate restore)

### Diff Format

Changes are stored as standard unified diffs, the same format tools like
`diff -u` and `git diff` produce:

- **Modified files**: unified diff with context lines
- **New files**: diff from `/dev/null` to `b/<path>` (full content included)
- **Deleted files**: diff from `a/<path>` to `/dev/null` (full content included)
- **Binary files**: noted as binary with the file's full content stored
  base64-encoded in the diff

The diff is generated entirely in Rust (e.g. `similar` crate's unified
diff output). No external `diff` or `git` command is invoked.

### Ignore Rules

When walking the project tree, decree skips:

- `.decree/` — always excluded (decree's own runtime data)
- `.git/` — always excluded (not decree's concern)
- Patterns from `.gitignore` — parsed using the `ignore` crate if the file
  exists (works without git being installed; it's just a text file)
- Patterns from `.decreeignore` — optional project-specific overrides,
  same glob syntax as `.gitignore`

The `ignore` crate (by BurntSushi) handles nested `.gitignore` files,
negation patterns, and all standard gitignore semantics.

### Creating a Checkpoint

Before a message's routine runs:

1. Walk the project tree (respecting ignore rules)
2. For each file: read contents, compute SHA-256, record size and mode
3. Write `manifest.json` to `.decree/runs/<chain>-<seq>/`

After the routine completes (success or failure):

1. Walk the project tree again
2. Compare against the pre-execution manifest:
   - Files in post but not in pre -> **new files**
   - Files in pre but not in post -> **deleted files**
   - Files in both but with different hashes -> **modified files**
3. For each changed file, generate a unified diff
4. Concatenate all diffs into a single `changes.diff`
5. Write to `.decree/runs/<chain>-<seq>/`

### Reverting to a Checkpoint

To undo a message's changes (revert to pre-execution state):

1. Read the message's `changes.diff` and `manifest.json`
2. Apply the diff in reverse:
   - Modified files: apply the reverse patch
   - New files (added by the routine): delete them
   - Deleted files (removed by the routine): restore from the diff content
3. Verify the resulting tree matches the manifest (SHA-256 hash check on
   all affected files)

Reverts must be perfect. After reverting, every affected file must match
its manifest hash exactly. Decree is the sole modifier of the working tree
during routine execution, so the reverse patch always applies cleanly
against the post-execution state. A failed revert is a hard error — it
indicates data corruption or external interference, not a recoverable
condition.

### Execution Flow Integration

```
for each message to process:
    create .decree/runs/<chain>-<seq>/
    copy inbox message -> <msg-dir>/message.md

    1. Save checkpoint -> <msg-dir>/manifest.json
    2. Execute routine (shell script via bash, or notebook via papermill)
    3. On SUCCESS:
       - Generate changes.diff -> <msg-dir>/changes.diff
       - If type=spec: mark spec as processed
    4. On FAILURE (not last retry):
       - Generate changes.diff -> <msg-dir>/changes.diff (partial work)
       - Leave changes in place (do NOT revert)
       - Retry — the AI can see and learn from its mistakes
    5. On FAILURE (last retry):
       - Generate changes.diff (partial work)
       - Revert to original checkpoint (pre-first-attempt)
       - Write failure-context.md summarizing prior attempts
       - Retry with clean slate + failure context
    6. On EXHAUSTION (all retries failed):
       - Revert to original checkpoint
       - Dead-letter the message (move to inbox/dead/)
       - Continue to next message
```

### Checkpoint Integrity

After every revert, decree verifies the affected files by comparing their
SHA-256 hashes against the target manifest. If any file doesn't match,
decree raises a hard error with the mismatched paths. No routine execution
should leave the working tree in a broken state — a hash mismatch after
revert indicates external interference or a bug in decree itself.

### No External Dependencies

The checkpoint system must not shell out to any external tool:
- No `git`, `diff`, `patch`, `rsync`, or any other CLI tool
- All file walking: `walkdir` crate
- All diffing: `similar` or `diffy` crate
- All hashing: `sha2` crate
- All ignore parsing: `ignore` crate
- Works identically whether or not the project is a git repository

## Acceptance Criteria

- **Given** a message is about to be executed
  **When** the routine has not yet started
  **Then** a `manifest.json` is saved to the message directory with SHA-256
  hashes of all project files (respecting ignore rules)

- **Given** a routine completes successfully
  **When** post-execution diff generation runs
  **Then** a `changes.diff` is saved to the message directory containing
  unified diffs for all new, modified, and deleted files

- **Given** a routine fails during execution
  **When** the failure is detected
  **Then** a `changes.diff` is still generated (capturing partial work)
  **And** the working tree is reverted to the manifest state

- **Given** a routine added new files during execution
  **When** the revert is triggered
  **Then** those new files are deleted and the tree matches the manifest

- **Given** a routine deleted files during execution
  **When** the revert is triggered
  **Then** those files are restored from the diff content

- **Given** all retries for a message are exhausted
  **When** exhaustion is reached
  **Then** the working tree is reverted to the message's manifest checkpoint

- **Given** no `.git/` directory exists in the project
  **When** checkpoints are created
  **Then** the checkpoint system works identically — no git dependency

- **Given** a `.gitignore` file exists in the project
  **When** the manifest is generated
  **Then** files matching gitignore patterns are excluded from the manifest

- **Given** a revert completes
  **When** integrity verification runs
  **Then** all affected files are hash-checked against the target manifest
  **And** any mismatch raises a hard error (not a warning)

- **Given** a `changes.diff` exists for a completed message
  **When** inspected with standard tools (e.g. `cat`, `less`, `diff` viewers)
  **Then** the content is human-readable unified diff format
