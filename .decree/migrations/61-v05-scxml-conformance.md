---
routine: rust-develop
---
# 61: v0.5 M3.6 SCXML conformance fixes

## Overview

Two places where decree's behaviour differed from SCXML's: overlapping event names in one state, and what an `onentry` failure skips. Part of the 0.5.0 rewrite specified in `docs/0.5-spec.md` (ticket M3.6).

## Requirements

Read spec sections 5 (SCXML subset, Rules, Validation V21), 6 (Events from an invoke: the `onentry` rules) first.

Add V21 to the validator, with a passing and a failing fixture. Change entry in `src/interpreter.rs` so a failing `onentry` script stops only its own state's block: the other states being entered still run theirs, outermost first, and then `error` is selected from the atomic state like any other event (bubbling to ancestors, `failed` if unhandled), without running the invoke. Root `onentry` failures follow the same path instead of targeting `failed` directly. Keep the root-final rule: a failing `onentry` on a root-level final state moves the run to `failed`. Update the interpreter's order tests (`order_onentry_failure_*`, `order_root_onentry_failure_*`) to the new behaviour, and the decree skill's `reference/scripts.md` (in `src/templates/skills/`, `.claude/skills/` and `.github/skills/`) to section 6's wording.

- Only this migration's scope; the other v0.5 migrations cover the rest of spec section 11.
- If the code does something the spec does not cover, or the spec is ambiguous here, do not guess: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes. The routine fails the run when `STOP` exists.
- Tests build their own `.decree/` in a temp directory and never touch this repository's `.decree/`. No test calls a real LLM or the network. No new dependencies unless the spec names them.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass. The routine checks this itself after you finish.

## Files to Modify

- src/machine.rs
- src/interpreter.rs
- tests/fixtures/check/
- src/templates/skills/decree/reference/scripts.md
- .claude/skills/decree/reference/scripts.md
- .github/skills/decree/reference/scripts.md

## Acceptance Criteria

- **Given** a machine with a state whose transitions include `done` and `done.state.work`
  **When** `decree check` runs
  **Then** it fails V21 naming the state and both events

- **Given** `mock/`, every project in `examples/`, and a fresh `decree init`
  **When** `decree check` runs
  **Then** it exits 0 with no warning

- **Given** a transition into compound `outer` (onentry `a`, exits 1) whose initial is `inner` (onentry `b`, invoke `work`)
  **When** the run steps
  **Then** `a` runs, `b` runs, `work` does not, and the `error` transition is selected from `inner`

- **Given** a failing root `onentry` script and a root-level `error` transition to `cleanup`
  **When** the run starts
  **Then** it takes that transition; with no `error` transition it ends in `failed`

- **Given** a root-level final state `done` whose `onentry` script fails
  **When** the run enters it
  **Then** the run ends in `failed`, as before
