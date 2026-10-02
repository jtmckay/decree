---
routine: rust-develop
---
# 50: v0.5 M4.2 Interrupts, run status and lock

## Overview

decree never continues a run on its own: signals and crashes leave it interrupted until `decree retry`. Part of the 0.5.0 rewrite specified in `docs/0.5-spec.md` (ticket M4.2).

## Requirements

Read spec sections 1 and 4 (Lifecycle step 6, Source of truth, Run status, Stopping, Run lock), 7 (step 1), 8 (retry row) first.

Implement run status, the `interrupted` event for signals and for crashes found at startup, the run lock, and continuing `pending` runs, as sections 4 and 7 describe.

- Only this migration's scope; the other v0.5 migrations cover the rest of spec section 11.
- If the code does something the spec does not cover, or the spec is ambiguous here, do not guess: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes. The routine fails the run when `STOP` exists.
- Tests build their own `.decree/` in a temp directory and never touch this repository's `.decree/`. No test calls a real LLM or the network. No new dependencies unless the spec names them.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass. The routine checks this itself after you finish.

## Files to Modify

- src/interpreter.rs
- src/message.rs

## Acceptance Criteria

- **Given** a sleeping invoke
  **When** decree receives SIGTERM
  **Then** the child is gone within 10 s, the last event is `interrupted` with `cause: signal`, no `onexit` script ran, and the exit code is 130

- **Given** decree killed with SIGKILL during a sleeping invoke
  **When** `decree process` runs again
  **Then** the run gets an `interrupted` event with `cause: crash` and is not continued

- **Given** that interrupted run
  **When** `decree retry <id>` then `decree process` run
  **Then** it continues at the recorded state with root `onentry` and the state's `onentry` scripts re-run

- **Given** an interrupted migration and a later pending migration
  **When** `decree process` runs
  **Then** the later one does not start, and the exit code is 1 with the `retry` command in the message

- **Given** a lock holding a live pid
  **When** decree starts
  **Then** the run is `active` and skipped

- **Given** a run whose last event is `waiting` and that has no lock
  **When** decree starts
  **Then** it is `waiting`, not marked as a crash

- **Given** a `message.md` mirror that disagrees with `events.jsonl`
  **When** decree touches the run
  **Then** the mirror is rewritten
