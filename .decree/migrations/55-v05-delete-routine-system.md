---
routine: rust-develop
---
# 55: v0.5 M5.3 Delete the old routine system

## Overview

Remove what machines replaced. Part of the 0.5.0 rewrite specified in `docs/0.5-spec.md` (ticket M5.3).

## Requirements

Read spec sections 1 and 10 (items 1–3, 8–12, 16, 17, 20) first.

Delete section 10 items 1–3, 8–12, 16, 17 and 20 after the rg check, plus `src/templates/router.md` and the 0.4 skill templates. Remove the `#![allow(dead_code)]` from M0.4 and fix what it hid.

- Only this migration's scope; the other v0.5 migrations cover the rest of spec section 11.
- If the code does something the spec does not cover, or the spec is ambiguous here, do not guess: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes. The routine fails the run when `STOP` exists.
- Tests build their own `.decree/` in a temp directory and never touch this repository's `.decree/`. No test calls a real LLM or the network. No new dependencies unless the spec names them.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass. The routine checks this itself after you finish.

## Files to Modify

- src/hooks.rs
- src/routine.rs
- src/commands/routine.rs
- src/commands/routine_sync.rs
- src/commands/skill.rs
- src/templates/
- src/config.rs
- src/error.rs
- src/lib.rs
- Cargo.toml

## Acceptance Criteria

- **Given** the updated crate
  **When** rg runs for the qualified form of every symbol in those rows (for example `routine::levenshtein`, `fn run_precheck`, `HookType`; see `docs/0.5-inventory.md`, C7)
  **Then** there are no hits

- **Given** the updated crate
  **When** `cargo tree -i inquire` and `cargo tree -i walkdir` run
  **Then** neither package is found

- **Given** the allow removed
  **When** `cargo build` runs
  **Then** there are no warnings and all tests pass
