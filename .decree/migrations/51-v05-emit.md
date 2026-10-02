---
routine: rust-develop
---
# 51: v0.5 M4.3 decree emit, decree event, replies and the cron writer

## Overview

Replace the outbox relay with one safe writer, used for emitted messages, replies to waiting runs and cron. Part of the 0.5.0 rewrite specified in `docs/0.5-spec.md` (ticket M4.3).

## Requirements

Read spec sections 1 and 4 (Lifecycle step 1, Replies), 8 (emit and event rows), 10 (item 5) first.

Add `decree emit` and `decree event`, and reply delivery (section 4, Replies), including `timeout_s` deadlines. Make cron write through the same temp-file-and-rename writer, with `trigger: cron`. Delete section 10 item 5 after the rg check.

- Only this migration's scope; the other v0.5 migrations cover the rest of spec section 11.
- If the code does something the spec does not cover, or the spec is ambiguous here, do not guess: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes. The routine fails the run when `STOP` exists.
- Tests build their own `.decree/` in a temp directory and never touch this repository's `.decree/`. No test calls a real LLM or the network. No new dependencies unless the spec names them.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass. The routine checks this itself after you finish.

## Files to Modify

- src/commands/emit.rs — new
- src/commands/event.rs — new
- src/cli.rs
- src/cron.rs
- src/message.rs
- src/commands/process.rs

## Acceptance Criteria

- **Given** an emit in progress
  **When** `inbox/` is listed
  **Then** no partial `*.md` is visible

- **Given** `DECREE_MACHINE` and `DECREE_STATE` for a state without that machine in `emits`
  **When** `decree emit` runs
  **Then** it exits 1

- **Given** an emitting run at depth `max_depth`
  **When** `decree emit` runs
  **Then** it exits 1

- **Given** an emitting run
  **When** `decree emit` succeeds
  **Then** `inbox/<id>.md` has `parent`, `depth` and `trigger: emit`

- **Given** a waiting run and a reply naming its wait id and one of its options
  **When** the reply is claimed
  **Then** the run continues with `source: person` and the reply is in `received/`

- **Given** a reply with a stale wait id or an event that is not an option
  **When** it is claimed
  **Then** the run stays waiting and the reply ends as a failed `invalid_message` run

- **Given** a `choose: person` with `timeout_s: 1`
  **When** the next pass runs after the deadline
  **Then** a `received` event for `error` with `timed_out: true` continues the run

- **Given** a run that is not waiting
  **When** `decree event` targets it
  **Then** it exits 1
