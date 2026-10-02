---
routine: rust-develop
---
# 52: v0.5 M4.4 init, process, daemon, status, tail, retry

## Overview

Wire the new pieces into the CLI. The daemon stops having its own copy of the pipeline. Part of the 0.5.0 rewrite specified in `docs/0.5-spec.md` (ticket M4.4).

## Requirements

Read spec sections 1 and 3, 8, 10 (items 13, 19, 21), 12 first.

Implement section 8 for `init`, `process` (with `--dry-run`), `daemon` (with `--interval`, calling the same functions as `process`), `status` (including the running script, its pid, elapsed time and log path), `tail` and `retry`. Delete section 10 items 13, 19 and 21 after the rg check.

- Only this migration's scope; the other v0.5 migrations cover the rest of spec section 11.
- If the code does something the spec does not cover, or the spec is ambiguous here, do not guess: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes. The routine fails the run when `STOP` exists.
- Tests build their own `.decree/` in a temp directory and never touch this repository's `.decree/`. No test calls a real LLM or the network. No new dependencies unless the spec names them.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass. The routine checks this itself after you finish.

## Files to Modify

- src/cli.rs
- src/main.rs
- src/commands/init.rs
- src/commands/process.rs
- src/commands/daemon.rs
- src/commands/status.rs
- src/commands/tail.rs — new
- src/commands/retry.rs — new
- src/commands/log.rs — delete
- src/commands/cron_list.rs — delete

## Acceptance Criteria

- **Given** an empty directory
  **When** `decree init`, `decree emit`, `decree process`, then `decree status <id>` run
  **Then** the run shows `done`

- **Given** an empty directory
  **When** `decree init` runs
  **Then** `.decree/` contains exactly the section 3 entries

- **Given** a failed run
  **When** `decree retry <id>` then `decree process` run
  **Then** it resumes at the retried state

- **Given** pending messages
  **When** `decree process --dry-run` runs
  **Then** they are listed and nothing runs

- **Given** a run whose script sleeps 5 s while printing a line each second
  **When** `decree status` and `decree tail` run during it
  **Then** status shows the script, its pid, elapsed time and log path; tail prints each line as it is written and exits when the run finishes

- **Given** a run that reaches a `choose: person` state
  **When** `decree process` runs
  **Then** it prints the wait id, the options and a `decree event` command per option, and exits 0; after `decree event` and another `decree process`, the run is `done`

- **Given** SIGINT during a run under `decree process`
  **When** decree stops
  **Then** the exit code is 130, the run is `interrupted`, and a later `decree process` leaves it alone

- **Given** a finished run
  **When** `decree status <id>` runs
  **Then** it shows the transitions, each script with its duration, and router decisions

- **Given** the updated crate
  **When** `rg -n 'fn process_single_message' src` runs
  **Then** there is at most one hit
