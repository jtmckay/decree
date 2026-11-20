---
routine: rust-develop
---
# 48: v0.5 M3.5 Escalation conditions

## Overview

Two `check` conditions that an escalation ladder needs: a regex on a string `data` value (a file name from `params`), and a test on the confidence of an earlier `choose: model` decision. `mock/` already uses both (`sort_document`), so the mock tests fail until this lands. Part of the 0.5.0 rewrite specified in `docs/0.5-spec.md` (ticket M3.5).

## Requirements

Read spec sections 5 (Conditions, Escalation, Rules V10), 7 (events.jsonl: decision), 9 (graph notes); `mock/.decree/machines/sort_document.yml` and its run `mock/.decree/runs/20261001T170412Z-3f9a51/` first.

Extend the condition type in `src/cond.rs`: `matches` is a subject on its own (unchanged) or the operator of a `string` `data` subject; add the `confidence: <state>` subject with the six comparison operators on numbers from 0 to 1 (floats are needed only here). Evaluate `confidence` from the latest `decision` event of that state in the run's `events.jsonl`, as 0 when it has no `confidence`. Update V10 as section 5 now words it, and the graph notes (`data file matches '<regex>'`, `confidence big_model at_least 0.4`). Delete `tests/fixtures/yaml/` and `test_yaml_fixtures_match_expected` in `src/message.rs`: they were snapshots proving the serde_yaml to serde_norway switch changed nothing, which is done, and they make every mock edit a fixture edit. Keep the small YAML 1.2 tests next to it (`on`, `yes`, `no` stay strings). Change nothing else in `mock/`.

- Only this migration's scope; the other v0.5 migrations cover the rest of spec section 11.
- If the code does something the spec does not cover, or the spec is ambiguous here, do not guess: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes. The routine fails the run when `STOP` exists.
- Tests build their own `.decree/` in a temp directory and never touch this repository's `.decree/`. No test calls a real LLM or the network. No new dependencies unless the spec names them.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass. The routine checks this itself after you finish.

## Files to Modify

- src/cond.rs
- src/machine.rs (validation)
- src/graph.rs
- src/interpreter.rs
- src/message.rs (delete the snapshot test)
- tests/
- tests/fixtures/yaml/ (delete)

## Acceptance Criteria

- **Given** `mock/`
  **When** `decree check` and `decree graph` run on a copy of it
  **Then** check exits 0 with no warning, and graph reproduces `mock/.decree/graph/` byte for byte, including `sort_document.md` and `local_router.md`

- **Given** a machine with `check: { data: file, matches: '\.md$' }` and `file` set by `params`
  **When** the run reaches that state
  **Then** it takes `yes` for `notes/a.md` and `no` for `notes/a.txt`

- **Given** a run whose `events.jsonl` has a `decision` event for state `big_model` with confidence 0.55, and one without a confidence
  **When** `check: { confidence: big_model, at_least: 0.4 }` is evaluated
  **Then** it gives `yes` for the first and `no` for the second (0)

- **Given** `matches` on `int` data, `confidence` naming a script state, and `at_least: 1.5` on `confidence`
  **When** `decree check` runs
  **Then** each fails V10 with the rule's message

- **Given** the whole suite
  **When** `cargo test` runs
  **Then** it passes, including the mock tests that fail before this migration
