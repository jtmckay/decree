---
routine: rust-develop
---
# 40: v0.5 M1.2 cond parser and evaluator

## Overview

Transitions out of router states can carry a one-comparison SCXML `cond`. Part of the 0.5.0 rewrite specified in `docs/0.5-spec.md` (ticket M1.2).

## Requirements

Read spec sections 1 and 5 (SCXML subset; Rules: cond, visits) first.

Create `src/cond.rs`: parse a `cond` under the section 5 grammar and evaluate it against `data` and visits.

- Only this migration's scope; the other v0.5 migrations cover the rest of spec section 11.
- If the code does something the spec does not cover, or the spec is ambiguous here, do not guess: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes. The routine fails the run when `STOP` exists.
- Tests build their own `.decree/` in a temp directory and never touch this repository's `.decree/`. No test calls a real LLM or the network. No new dependencies unless the spec names them.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass. The routine checks this itself after you finish.

## Files to Modify

- src/cond.rs — new
- src/lib.rs

## Acceptance Criteria

- **Given** each operator and each operand kind
  **When** a `cond` using it is evaluated
  **Then** the result matches a hand-computed value (one test each)

- **Given** `a && b`, `visits.x <`, `data.y == 1 == 2`, `foo == 1`, `exit_code == 0` or the empty string
  **When** it is parsed
  **Then** parsing fails
