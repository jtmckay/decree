---
routine: rust-develop
---
# 49: v0.5 M4.1 Messages, claim and the migration queue

## Overview

One message type for inbox, cron and migrations, with the migration rules kept from 0.4.2. Part of the 0.5.0 rewrite specified in `docs/0.5-spec.md` (ticket M4.1).

## Requirements

Read spec sections 1 and 4, 10 (items 4, 6, 7) first.

Implement message parse and write, inbox claim, validation, and the six section 4 migration rules with the `processed.md` ledger. Then delete section 10 items 4, 6 and 7 after the rg check.

- Only this migration's scope; the other v0.5 migrations cover the rest of spec section 11.
- If the code does something the spec does not cover, or the spec is ambiguous here, do not guess: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes. The routine fails the run when `STOP` exists.
- Tests build their own `.decree/` in a temp directory and never touch this repository's `.decree/`. No test calls a real LLM or the network. No new dependencies unless the spec names them.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass. The routine checks this itself after you finish.

## Files to Modify

- src/message.rs
- src/commands/process.rs
- src/config.rs

## Acceptance Criteria

- **Given** a message with unknown keys and CRLF line endings
  **When** it is read and written back
  **Then** unknown keys, key order and body bytes are unchanged

- **Given** a BOM, or trailing spaces on the fences
  **When** the message is parsed
  **Then** its frontmatter is read normally

- **Given** an opening fence with no closing fence
  **When** the message is parsed
  **Then** it fails naming the file and line

- **Given** two threads
  **When** they claim the same inbox file
  **Then** exactly one succeeds

- **Given** a message naming an unknown machine
  **When** it is processed
  **Then** it ends `failed` with an `invalid_message` transition event

- **Given** a message with `routine: x`
  **When** it is read
  **Then** its machine is `x`

- **Given** two migrations where the first fails
  **When** `decree process` runs
  **Then** the second never starts, `processed.md` is unchanged, and the exit code is 1

- **Given** a pending migration with invalid frontmatter
  **When** `decree process` runs
  **Then** nothing runs, every error is printed, and the exit code is 1

- **Given** a migration listed in `processed.md`
  **When** `decree process` runs
  **Then** it is skipped

- **Given** a migration that emits a follow-up
  **When** `decree process` runs
  **Then** the follow-up finishes before the next migration starts

- **Given** a migration that reaches `done` with `onentry: [commit]`
  **When** the commit script runs
  **Then** `processed.md` already lists it

- **Given** a final-state `onentry` script that fails
  **When** the migration finishes
  **Then** its `processed.md` line is removed again
