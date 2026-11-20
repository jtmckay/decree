---
routine: rust-develop
---
# 36: v0.5 M0.3 Delete the error::color wrappers

## Overview

`error::color` wraps `colored` in seven pass-through functions. Call `colored` directly. Part of the 0.5.0 rewrite specified in `docs/0.5-spec.md` (ticket M0.3).

## Requirements

Read spec sections 1 and 10 (item 14) first.

Delete the `color` module from `src/error.rs` and update every call site.

- Only this migration's scope; the other v0.5 migrations cover the rest of spec section 11.
- If the code does something the spec does not cover, or the spec is ambiguous here, do not guess: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes. The routine fails the run when `STOP` exists.
- Tests build their own `.decree/` in a temp directory and never touch this repository's `.decree/`. No test calls a real LLM or the network. No new dependencies unless the spec names them.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass. The routine checks this itself after you finish.

## Files to Modify

- src/error.rs
- call sites (rg `color::`)

## Acceptance Criteria

- **Given** the updated crate
  **When** `rg -n 'color::' src` runs
  **Then** no hits remain

- **Given** `NO_COLOR=1` is set
  **When** `decree status` runs
  **Then** the output has no ANSI escape bytes
