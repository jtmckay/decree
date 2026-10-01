---
routine: rust-develop
---
# 37: v0.5 M0.4 Make modules pub(crate)

## Overview

Every internal function is public API today. Shrink the surface before the larger changes. Part of the 0.5.0 rewrite specified in `docs/0.5-spec.md` (ticket M0.4).

## Requirements

Read spec sections 1 and 10 (item 18) first.

Make every module in `src/lib.rs` `pub(crate)` (or move the binary to not need a lib), exposing only what `src/main.rs` needs. Move tests that need internals into unit tests or `tests/` against the binary. List new dead-code warnings in `docs/0.5-inventory.md` under **Dead code**, and add a crate-level `#![allow(dead_code)] // removed in M5.3`. Fix nothing else.

- Only this migration's scope; the other v0.5 migrations cover the rest of spec section 11.
- If the code does something the spec does not cover, or the spec is ambiguous here, do not guess: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes. The routine fails the run when `STOP` exists.
- Tests build their own `.decree/` in a temp directory and never touch this repository's `.decree/`. No test calls a real LLM or the network. No new dependencies unless the spec names them.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass. The routine checks this itself after you finish.

## Files to Modify

- src/lib.rs
- src/main.rs
- tests/integration_test.rs
- docs/0.5-inventory.md

## Acceptance Criteria

- **Given** the updated crate
  **When** `cargo test` runs
  **Then** all tests pass

- **Given** the updated crate
  **When** `cargo clippy --all-targets -- -D warnings` runs
  **Then** it passes
