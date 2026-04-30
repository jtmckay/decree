---
routine: rust-develop
---

# 28: Stop processing further migrations on dead-letter

## Overview

Currently, when a migration's inbox message exhausts all retries and is
dead-lettered, the outer migration loop silently continues to the next
migration. Change this so the loop stops immediately and exits non-zero.

## Requirements

`drain_inbox` currently returns `Result<(), DecreeError>`. Introduce a
`DrainResult` struct with a `dead_lettered: bool` field. Return it from
`drain_inbox` instead of `()`.

In the outer migration processing loop (in `run`), after calling
`drain_inbox` with a chain prefix, inspect `DrainResult::dead_lettered`.
If it is `true`, stop the migration loop, print an error message, and
return `Err` so the process exits non-zero.

The error message must identify which migration failed:

```
[Migration 3/5: 03-add-auth.md] FAILED — stopping. Fix the migration or
dead-letter it manually, then re-run `decree process`.
```

`DrainResult::dead_lettered` is set to `true` when at least one message
processed during that drain call was moved to `inbox/dead/`. It is set to
`false` when all messages succeeded.

The `drain_inbox` loop already dead-letters messages on unexpected errors
(line: `let _ = dead_letter(project_root, &filename)`). Those paths must
also set `dead_lettered = true` in the returned result.

Inbox-only runs (calls to `drain_inbox` with `prefer_chain = None` at the
end of the migration loop) are unaffected — dead-lettering during a
chain-less inbox drain does not stop the migration loop.

## Files to Modify

- `src/commands/process.rs` — introduce `DrainResult`; update `drain_inbox`
  signature and all call sites; stop migration loop on
  `DrainResult::dead_lettered`

## Acceptance Criteria

- **Given** a migration whose routine always exits non-zero and `max_retries`
  is exhausted
  **When** `decree process` runs
  **Then** processing stops after the failed migration, no subsequent
  migrations are started, and the process exits non-zero

- **Given** a migration that fails and is dead-lettered, followed by a second
  unprocessed migration
  **When** `decree process` runs
  **Then** the second migration is never started

- **Given** all migrations succeed
  **When** `decree process` runs
  **Then** all migrations are processed and the process exits 0 (unchanged
  behaviour)

- **Given** a message dropped directly into the inbox (not from a migration)
  that is dead-lettered
  **When** `decree process` drains the remaining inbox after all migrations
  **Then** the migration loop does not stop (inbox-only dead-letters are
  unaffected)
