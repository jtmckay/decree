---
routine: rust-develop
---
# 60: v0.5 M5.7 Port the examples and the blackbox test

## Overview

The sample projects in `examples/` and `blackbox_test/test_decree.sh` are written for 0.4 (routines, hooks, `routine:`, `ai_router`). Bring them to 0.5 so nothing in the repository describes the old system. Part of the 0.5.0 rewrite specified in `docs/0.5-spec.md` (ticket M5.7).

## Requirements

Read spec sections 1 and 3, 4, 5, 6, 8; `mock/` as the model of a 0.5 project first.

For each project in `examples/*/`: turn every routine in `.decree/routines/` into a machine in `.decree/machines/` plus scripts in `.decree/scripts/` (one script per step; split what a routine did in one file), with the `# Graph:` first line; replace `config.yml` with the 0.5 keys; change `routine:` to `machine:` in its migrations and cron files (these are sample inputs, not this repository's history); remove `processed.md` and run folders so each example starts fresh; run `decree graph` and commit `.decree/graph/`; update the example's README to 0.5 commands. Keep each example's purpose and content. For `examples/decree/README.md`, which tells how decree was built with itself, keep the history but fix any command that no longer exists. Rewrite `blackbox_test/test_decree.sh` against the 0.5 CLI (init, emit, process, status, tail, retry, event, check, graph); if `tests/` already covers a case end to end, drop it from the script and say so in its header.

- Only this migration's scope; the other v0.5 migrations cover the rest of spec section 11.
- If the code does something the spec does not cover, or the spec is ambiguous here, do not guess: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes. The routine fails the run when `STOP` exists.
- Tests build their own `.decree/` in a temp directory and never touch this repository's `.decree/`. No test calls a real LLM or the network. No new dependencies unless the spec names them.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass. The routine checks this itself after you finish.

## Files to Modify

- examples/
- blackbox_test/test_decree.sh

## Acceptance Criteria

- **Given** each project in `examples/`
  **When** `decree check` runs in it
  **Then** it exits 0 with no warning

- **Given** each project in `examples/`
  **When** `rg -n -w 'routine|routines|outbox|hooks|ai_router' examples/` runs
  **Then** nothing matches except prose that explains the 0.4 history in `examples/decree/README.md`

- **Given** the rewritten blackbox script
  **When** it runs against the built binary
  **Then** it passes

- **Given** every example README
  **When** its commands are run as written in that example
  **Then** each succeeds (commands that need an AI backend may use a stub on `PATH`)
