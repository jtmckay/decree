---
routine: rust-develop
---
# 54: v0.5 M5.1 Port develop and rust-develop

## Overview

Built-in routines become machines plus bash scripts. Claude-specific waits move out of core. Part of the 0.5.0 rewrite specified in `docs/0.5-spec.md` (ticket M5.1).

## Requirements

Read spec sections 1 and 5, 6, 13; `mock/` for the target shape; `docs/0.5-inventory.md` first.

Port `src/templates/develop.sh` and `src/templates/rust-develop.sh` to machines `develop` and `rust_develop` with scripts that `decree init` writes. Move Claude token-exhaustion detection, waiting until reset, and session resume (migrations 29–30, `process.rs`) into those scripts. Read 0.4.2 behaviour from the baseline commit in `docs/0.5-inventory.md`.

- Only this migration's scope; the other v0.5 migrations cover the rest of spec section 11.
- If the code does something the spec does not cover, or the spec is ambiguous here, do not guess: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes. The routine fails the run when `STOP` exists.
- Tests build their own `.decree/` in a temp directory and never touch this repository's `.decree/`. No test calls a real LLM or the network. No new dependencies unless the spec names them.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass. The routine checks this itself after you finish.

## Files to Modify

- src/templates/
- src/commands/init.rs

## Acceptance Criteria

- **Given** each ported machine
  **When** `decree check` runs
  **Then** it passes

- **Given** the same input message
  **When** 0.4.2's routine and the ported machine run with a stubbed `claude`
  **Then** both end in the same outcome

- **Given** a stubbed `claude` that prints a usage-limit message with a reset time
  **When** the ported script runs
  **Then** it waits until the reset, then resumes the session
