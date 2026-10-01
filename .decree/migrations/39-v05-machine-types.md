---
routine: rust-develop
---
# 39: v0.5 M1.1 Machine types and loader

## Overview

Add the machine model: YAML statecharts loaded into an arena. Not wired into any command yet. Part of the 0.5.0 rewrite specified in `docs/0.5-spec.md` (ticket M1.1).

## Requirements

Read spec sections 1 and 3 (Shared machines), 5, 13 first.

Create `src/machine.rs` with the section 5 types and a loader for `machines/*.yml`, project-local first, then `shared_source`. Flatten into the arena section 5 describes. Save the three section 5 examples as `tests/fixtures/machines/{hello,deploy,feature}.yml`.

- Only this migration's scope; the other v0.5 migrations cover the rest of spec section 11.
- If the code does something the spec does not cover, or the spec is ambiguous here, do not guess: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes. The routine fails the run when `STOP` exists.
- Tests build their own `.decree/` in a temp directory and never touch this repository's `.decree/`. No test calls a real LLM or the network. No new dependencies unless the spec names them.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass. The routine checks this itself after you finish.

## Files to Modify

- src/machine.rs — new
- src/lib.rs
- tests/fixtures/machines/feature.yml — new

## Acceptance Criteria

- **Given** the three section 5 examples
  **When** they are loaded
  **Then** loading succeeds and `feature`'s arena has 9 states plus the root

- **Given** a machine with a misspelled key
  **When** it is loaded
  **Then** the error reads `machines/<id>.yml: <state path>: unknown field`

- **Given** the same machine id project-local and in `shared_source`
  **When** machines are loaded
  **Then** the project-local one is used
