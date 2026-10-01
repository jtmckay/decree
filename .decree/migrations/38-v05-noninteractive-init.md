---
routine: rust-develop
---
# 38: v0.5 M0.5 Non-interactive init

## Overview

`decree init` asks questions on a TTY. 0.5 has no prompts; flags replace them. Part of the 0.5.0 rewrite specified in `docs/0.5-spec.md` (ticket M0.5).

## Requirements

Read spec sections 1 and 8 (init row), 10 (item 16) first.

Replace every prompt in `init` (backend selection, permissions file, overwrite confirmation) with the section 8 `--ai` and `--permissions` flags and the refuse-to-overwrite rule. Do not remove the `inquire` dependency yet: `routine`, `log` and `skill` still use it until M5.3.

- Only this migration's scope; the other v0.5 migrations cover the rest of spec section 11.
- If the code does something the spec does not cover, or the spec is ambiguous here, do not guess: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes. The routine fails the run when `STOP` exists.
- Tests build their own `.decree/` in a temp directory and never touch this repository's `.decree/`. No test calls a real LLM or the network. No new dependencies unless the spec names them.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass. The routine checks this itself after you finish.

## Files to Modify

- src/cli.rs
- src/commands/init.rs
- tests/integration_test.rs

## Acceptance Criteria

- **Given** an empty temp directory with stdin closed
  **When** `decree init` runs
  **Then** it completes with exit 0 and asks nothing

- **Given** a directory that already has `.decree/`
  **When** `decree init` runs
  **Then** it exits 2 and changes nothing

- **Given** `--ai claude`
  **When** `decree init` runs
  **Then** `commands.ai_router` uses the claude command
