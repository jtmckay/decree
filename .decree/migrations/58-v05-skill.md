---
routine: rust-develop
---
# 58: v0.5 M5.6 Decree skill for 0.5

## Overview

The decree skill teaches agents how to work in a decree project. Rewrite it for 0.5 and have `init` write it. Part of the 0.5.0 rewrite specified in `docs/0.5-spec.md` (ticket M5.6).

## Requirements

Read spec sections 1 and 8 (init row), 11 (M5.6); `mock/` first.

Rewrite `src/templates/skills/decree/` (`SKILL.md` plus `reference/`) around messages, machines, scripts, `decree check`, `decree graph` and `decree emit`, using `mock/` as its worked example. `decree init` writes it to `.claude/skills/decree/` (backend `claude`) or `.github/skills/decree/` (backend `copilot`) and never overwrites an existing file. Replace this repository's `.claude/skills/decree/` and `.github/skills/decree/` with the new copies.

- Only this migration's scope; the other v0.5 migrations cover the rest of spec section 11.
- If the code does something the spec does not cover, or the spec is ambiguous here, do not guess: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes. The routine fails the run when `STOP` exists.
- Tests build their own `.decree/` in a temp directory and never touch this repository's `.decree/`. No test calls a real LLM or the network. No new dependencies unless the spec names them.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass. The routine checks this itself after you finish.

## Files to Modify

- src/templates/skills/decree/
- src/commands/init.rs
- .claude/skills/decree/
- .github/skills/decree/

## Acceptance Criteria

- **Given** an empty directory
  **When** `decree init --ai claude` runs
  **Then** `.claude/skills/decree/SKILL.md` exists

- **Given** the new skill files
  **When** rg runs for `routine`, `outbox`, `hooks` and `router.md`
  **Then** there are no hits

- **Given** a directory with `.claude/skills/decree/SKILL.md` but no `.decree/`
  **When** `decree init --ai claude` runs
  **Then** the existing `SKILL.md` is unchanged
