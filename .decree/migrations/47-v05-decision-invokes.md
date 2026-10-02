---
routine: rust-develop
---
# 47: v0.5 M3.2 Decision invokes

## Overview

The machine model changed after migrations 39–46 ran: a state now invokes a script, a machine, or a built-in decision (`check`, `choose: model`, `choose: person`); transitions have no `cond`; router states and implicit waiting states are gone. Rework the code those migrations built to match. Running `machine` and `choose: model` invokes comes in the next migration. Part of the 0.5.0 rewrite specified in `docs/0.5-spec.md` (ticket M3.2).

## Requirements

Read spec sections 1 and 4 (Replies), 5 (all), 6 (Environment), 7 (Step loop, Check, Choose: person, events.jsonl), 9 (labels and notes); `mock/` first.

Rework `src/machine.rs`, `src/cond.rs` (typed condition objects replace the string grammar; rename the module if a better name fits), the validator, `src/graph.rs` and `src/interpreter.rs` to section 5's model: `invoke` as a script name or a `machine`/`check`/`choose` object; no `router: llm`, `default` or `cond` on states or transitions; the V1–V20 rules as now written; graph labels and decision notes; `decree graph` writing `.decree/graph/` as section 9 describes (one file per machine with a link back to its YAML, `system.md` with links, stale files removed) instead of printing; `decree check` warning when those files are out of date; `decision`, `waiting` and `received` events; `DECREE_EVENTS`, `DECREE_WAIT_ID`, `DECREE_QUESTION`, `DECREE_CHOICES`. Parse and validate `machine` and `choose: model` invokes; running them is M3.3. Remove the `Router` trait and `ScriptedRouter` if nothing else needs them. Update the fixtures in `tests/fixtures/` to the new syntax.

- Only this migration's scope; the other v0.5 migrations cover the rest of spec section 11.
- If the code does something the spec does not cover, or the spec is ambiguous here, do not guess: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes. The routine fails the run when `STOP` exists.
- Tests build their own `.decree/` in a temp directory and never touch this repository's `.decree/`. No test calls a real LLM or the network. No new dependencies unless the spec names them.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass. The routine checks this itself after you finish.

## Files to Modify

- src/machine.rs
- src/cond.rs
- src/interpreter.rs
- src/router.rs
- src/graph.rs
- src/commands/check.rs
- tests/fixtures/

## Acceptance Criteria

- **Given** `mock/` as the project root
  **When** `decree check` and `decree graph` run
  **Then** check exits 0 with no warning, and `decree graph` rewrites `mock/.decree/graph/` with no change to any file

- **Given** each condition subject and operator, including a `{ data: <name> }` value and `matches` on an `input` state
  **When** a `check` state runs
  **Then** it produces the right `yes` or `no` and a `decision` event

- **Given** a `choose: person` state
  **When** it is entered
  **Then** its `ask` script sees `DECREE_WAIT_ID` and `DECREE_CHOICES`, a `waiting` event is appended, and a `received` event continues the run with `source: person`

- **Given** a transition with `cond`, a state with `router: llm`, a `choose` without `question`, a `choose` option without `description`, or a machine that invokes itself
  **When** `decree check` runs
  **Then** each fails with the section 5 message

- **Given** the updated crate
  **When** `rg -n 'router: llm' src tests` runs
  **Then** it finds nothing outside fixtures for rejected input
