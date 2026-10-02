---
routine: rust-develop
---
# 57: v0.5 M5.5 README, help and 0.5.0

## Overview

Document the three building blocks and cut 0.5.0. Part of the 0.5.0 rewrite specified in `docs/0.5-spec.md` (ticket M5.5).

## Requirements

Read spec sections 1 and 2, 3, 4, 8, 13; `mock/README.md` first.

Rewrite `README.md` and `src/templates/help.txt` around messages, machines and scripts, and link `docs/routers.md` and `docs/services.md`. Set the version to 0.5.0. Make sure `decree check` passes with `mock/` as the project root.

- Only this migration's scope; the other v0.5 migrations cover the rest of spec section 11.
- If the code does something the spec does not cover, or the spec is ambiguous here, do not guess: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes. The routine fails the run when `STOP` exists.
- Tests build their own `.decree/` in a temp directory and never touch this repository's `.decree/`. No test calls a real LLM or the network. No new dependencies unless the spec names them.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass. The routine checks this itself after you finish.

## Files to Modify

- README.md
- src/templates/help.txt
- Cargo.toml

## Acceptance Criteria

- **Given** a fresh `decree init` in a temp directory
  **When** every README command runs as written
  **Then** each succeeds

- **Given** the updated crate
  **When** `decree --version` runs
  **Then** it prints 0.5.0

- **Given** `mock/` as the working directory
  **When** `decree check` runs
  **Then** it exits 0
