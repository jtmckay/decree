---
routine: rust-develop
---
# 55: v0.5 M5.2 Port hooks and git-stash templates

## Overview

Hooks become `onentry` and `onexit` scripts. Part of the 0.5.0 rewrite specified in `docs/0.5-spec.md` (ticket M5.2).

## Requirements

Read spec sections 1 and 5 (onentry and onexit), 6 (Environment, Attempts); `mock/.decree/scripts/git_baseline.sh` and `snapshot.sh` first.

Map `beforeAll` to root `onentry`, `afterAll` to root `onexit`, `beforeEach` to each atomic state's `onentry`, `afterEach` to each atomic state's `onexit`, and `onDeadLetter` to `failed`'s `onentry`. These run once per visit to a state (SCXML), not per attempt: `max_attempts` re-runs only the invoke. Replace `git-baseline.sh` and `git-stash-changes.sh` with the per-visit scripts in `mock/`, written by `decree init`: `git_baseline` (root `onentry`; records `HEAD` once, safe to repeat) and `snapshot` (`onentry` of a working state; stashes a checkpoint each visit). 0.4.2's "restore the baseline before the final attempt" is dropped: a machine that wants a clean retry loops back through a state, as `feature` does with rounds.

- Only this migration's scope; the other v0.5 migrations cover the rest of spec section 11.
- If the code does something the spec does not cover, or the spec is ambiguous here, do not guess: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes. The routine fails the run when `STOP` exists.
- Tests build their own `.decree/` in a temp directory and never touch this repository's `.decree/`. No test calls a real LLM or the network. No new dependencies unless the spec names them.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass. The routine checks this itself after you finish.

## Files to Modify

- src/templates/
- src/commands/init.rs
- tests/

## Acceptance Criteria

- **Given** the 0.4.2 hook tests rewritten as machines
  **When** they run
  **Then** scripts run in the order root `onentry`, `onentry`, invoke, `onexit`, root `onexit`, and with two attempts the invoke runs twice between one `onentry` and one `onexit`

- **Given** a run that ends `failed`, including one whose `onentry` failed
  **When** it finishes
  **Then** the former `onDeadLetter` script ran exactly once

- **Given** a project made by `decree init`
  **When** `git_baseline` and `snapshot` run in a git repository
  **Then** the baseline is written once, and each visit stores one stash
