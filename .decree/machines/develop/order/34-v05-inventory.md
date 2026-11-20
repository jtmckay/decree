---
routine: rust-develop
---
# 34: v0.5 M0.1 Inventory and lint baseline

## Overview

Record the 0.4.2 baseline that later v0.5 migrations compare against, and make the baseline pass the lint gate every later migration needs. No behaviour change. Part of the 0.5.0 rewrite specified in `docs/0.5-spec.md` (ticket M0.1).

## Requirements

Read spec sections 1 and 10, 11 (M0.1) first.

Write `docs/0.5-inventory.md` with: the output of `git rev-parse HEAD` before any change; for every symbol in spec section 10, `rg -n -w '<symbol>' src tests` with hit count and files; where signal handling kills child process groups; every use of `inquire` and `walkdir`; a **Conflicts** list of anything in the code that contradicts the spec. Then run `cargo fmt` and fix every `cargo clippy --all-targets -- -D warnings` finding, in `src/` and `tests/`, without changing behaviour. Do not change what any test asserts.

- Only this migration's scope; the other v0.5 migrations cover the rest of spec section 11.
- If the code does something the spec does not cover, or the spec is ambiguous here, do not guess: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes. The routine fails the run when `STOP` exists.
- Tests build their own `.decree/` in a temp directory and never touch this repository's `.decree/`. No test calls a real LLM or the network. No new dependencies unless the spec names them.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass. The routine checks this itself after you finish.

## Files to Modify

- docs/0.5-inventory.md — new
- src/ (formatting and clippy fixes only)

## Acceptance Criteria

- **Given** the repository at the baseline commit
  **When** this migration runs
  **Then** `docs/0.5-inventory.md` lists every section 10 symbol with a hit count and file list

- **Given** this migration has run
  **When** `cargo test` runs
  **Then** it passes with the same number of tests as before

- **Given** this migration has run
  **When** `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` run
  **Then** both pass
