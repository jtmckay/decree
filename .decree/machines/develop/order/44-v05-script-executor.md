---
routine: rust-develop
---
# 44: v0.5 M2.2 Script executor

## Overview

Run scripts with the section 6 environment, logs, attempts, timeout and event parsing. Part of the 0.5.0 rewrite specified in `docs/0.5-spec.md` (ticket M2.2).

## Requirements

Read spec sections 1 and 6 first.

Add the executor to `src/runtime.rs`, appending one `script` event per execution (section 7). Reuse 0.4.2's `truncate_log_if_needed` and its `process_group(0)` spawn. Do not reuse its signal handling: it never handles SIGTERM and never escalates to SIGKILL (`docs/0.5-inventory.md`, C2 and C3).

- Only this migration's scope; the other v0.5 migrations cover the rest of spec section 11.
- If the code does something the spec does not cover, or the spec is ambiguous here, do not guess: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes. The routine fails the run when `STOP` exists.
- Tests build their own `.decree/` in a temp directory and never touch this repository's `.decree/`. No test calls a real LLM or the network. No new dependencies unless the spec names them.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass. The routine checks this itself after you finish.

## Files to Modify

- src/runtime.rs
- tests/fixtures/scripts/ — new

## Acceptance Criteria

- **Given** an invoke that exits 0
  **When** it runs
  **Then** the event is `done`

- **Given** an invoke that exits 3
  **When** it runs with attempts exhausted
  **Then** the event is `error`

- **Given** an invoke whose last stdout line is `{"event":"pass"}`
  **When** it exits 0
  **Then** the event is `pass`

- **Given** the same invoke exiting 1
  **When** it runs
  **Then** the event is `error`

- **Given** a script writing to stderr
  **When** it runs
  **Then** its log lines carry `[stderr] `

- **Given** a script that prints its environment
  **When** it runs
  **Then** every section 6 variable is set

- **Given** a state with `timeout_s: 1` running `sleep 100`
  **When** it runs
  **Then** it ends with `error` and `timed_out: true` within 12 s

- **Given** a running `sleep 100` script
  **When** decree receives SIGTERM
  **Then** the child is gone within 10 s and no `script` event is written for it

- **Given** a `sleep 1` script
  **When** it runs
  **Then** its `script` event has every section 7 field and `duration_ms` between 1000 and 2000
