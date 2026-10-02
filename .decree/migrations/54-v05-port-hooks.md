---
routine: rust-develop
---
# 54: v0.5 M5.2 Port hooks and git-stash templates

## Overview

Hooks become `onentry` and `onexit` scripts. Part of the 0.5.0 rewrite specified in `docs/0.5-spec.md` (ticket M5.2).

## Requirements

Read spec sections 1 and 5 (onentry and onexit), 6 (Environment) first.

Map `beforeAll` to root `onentry`, `afterAll` to root `onexit`, `beforeEach` to each atomic state's `onentry`, `afterEach` to each atomic state's `onexit`, and `onDeadLetter` to `failed`'s `onentry`. Port `git-baseline.sh` and `git-stash-changes.sh` to scripts `decree init` writes, using `DECREE_ATTEMPT`, `DECREE_MAX_ATTEMPTS` and `DECREE_FINAL_ATTEMPT`.

- Only this migration's scope; the other v0.5 migrations cover the rest of spec section 11.
- If the code does something the spec does not cover, or the spec is ambiguous here, do not guess: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes. The routine fails the run when `STOP` exists.
- Tests build their own `.decree/` in a temp directory and never touch this repository's `.decree/`. No test calls a real LLM or the network. No new dependencies unless the spec names them.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass. The routine checks this itself after you finish.

## Files to Modify

- src/templates/git-baseline.sh
- src/templates/git-stash-changes.sh
- src/commands/init.rs

## Acceptance Criteria

- **Given** the 0.4.2 hook tests rewritten as machines
  **When** they run
  **Then** the scripts run in the same order as in 0.4.2

- **Given** a run that ends `failed`
  **When** it finishes
  **Then** the former `onDeadLetter` script ran exactly once
