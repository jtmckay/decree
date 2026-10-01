---
routine: rust-develop
---
# 48: v0.5 M3.3 Router backends (BLOCKED)

## Overview

BLOCKED on the router spike, `docs/spikes/router.md`. Do not run this migration until the spike's decision record is filled in and this file is rewritten from it. Part of the 0.5.0 rewrite specified in `docs/0.5-spec.md` (ticket M3.3).

## Requirements

Read spec sections 1 and 7 (Router); `docs/spikes/router.md` first.

Write `STOP` immediately, explaining that this migration is blocked on the router spike. Change nothing.

- Only this migration's scope; the other v0.5 migrations cover the rest of spec section 11.
- If the code does something the spec does not cover, or the spec is ambiguous here, do not guess: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes. The routine fails the run when `STOP` exists.
- Tests build their own `.decree/` in a temp directory and never touch this repository's `.decree/`. No test calls a real LLM or the network. No new dependencies unless the spec names them.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass. The routine checks this itself after you finish.

## Files to Modify

- none until rewritten

## Acceptance Criteria

- **Given** this placeholder
  **When** it runs
  **Then** it stops without changes
