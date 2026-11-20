---
routine: rust-develop
---
# 42: v0.5 M1.3 Validator and decree check

## Overview

Validate machines and pending messages before anything runs, and expose it as `decree check`. Part of the 0.5.0 rewrite specified in `docs/0.5-spec.md` (ticket M1.3).

## Requirements

Read spec sections 1 and 4 (Parsing and writing), 5 (Validation), 8 (check row) first.

Implement rules V1–V14 on the arena, using the resolver for V12, and M1–M3 using the frontmatter rules in section 4. Add the `check` subcommand. The 0.4 message code stays in place; M4.1 replaces it.

- Only this migration's scope; the other v0.5 migrations cover the rest of spec section 11.
- If the code does something the spec does not cover, or the spec is ambiguous here, do not guess: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes. The routine fails the run when `STOP` exists.
- Tests build their own `.decree/` in a temp directory and never touch this repository's `.decree/`. No test calls a real LLM or the network. No new dependencies unless the spec names them.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass. The routine checks this itself after you finish.

## Files to Modify

- src/machine.rs
- src/cli.rs
- src/main.rs
- src/commands/check.rs — new
- tests/fixtures/check/ — new

## Acceptance Criteria

- **Given** one valid and one invalid fixture per rule V1–V14 and M1–M3
  **When** `decree check` runs on each
  **Then** the valid one exits 0 and the invalid one exits 1 naming that rule's problem

- **Given** several invalid machines
  **When** `decree check` runs
  **Then** it prints one line per error
