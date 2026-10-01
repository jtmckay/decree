---
routine: rust-develop
---
# 45: v0.5 M3.1 Interpreter step loop, events.jsonl and visits

## Overview

Step a run through a machine with SCXML's exit and entry order: attempts, final states, router decisions against a test router, and the event log that state, status and visits are derived from. Composition and waiting states come in the next migration. Part of the 0.5.0 rewrite specified in `docs/0.5-spec.md` (ticket M3.1).

## Requirements

Read spec sections 1 and 4 (Lifecycle, Source of truth, Run status), 5 (Kinds of state, SCXML subset, Rules), 6 (Attempts), 7 (Step loop, Router steps 1–6, the Router trait, events.jsonl) first.

Create `src/interpreter.rs` with the section 7 step loop for atomic, compound (entered through `initial`), pass-through, invoke, router and root-level final states, writing `events.jsonl` and computing visits and run status from it. Define the `Router` trait, `RouterRequest`, `RouterReply` and `ScriptedRouter` exactly as section 7 shows, and implement router steps 1–6. Do not implement any real router backend: that waits for `docs/spikes/router.md`. Leave transitions on compound states, `type: internal`, nested final states and waiting states to the next migration. Not wired to `process` yet.

- Only this migration's scope; the other v0.5 migrations cover the rest of spec section 11.
- If the code does something the spec does not cover, or the spec is ambiguous here, do not guess: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes. The routine fails the run when `STOP` exists.
- Tests build their own `.decree/` in a temp directory and never touch this repository's `.decree/`. No test calls a real LLM or the network. No new dependencies unless the spec names them.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass. The routine checks this itself after you finish.

## Files to Modify

- src/interpreter.rs — new
- src/router.rs — new (trait and ScriptedRouter only)
- src/lib.rs
- tests/fixtures/machines/

## Acceptance Criteria

- **Given** a machine whose every script appends its name to one file
  **When** a normal path, a self-transition, leaving a compound state, an unhandled error, an `onentry` failure, an `onexit` failure, a final-state `onentry` failure, and reaching a final state are each run
  **Then** the file shows the exact section 7 order for each (root `onexit` last)

- **Given** a state with `max_attempts: 3` whose invoke fails twice then succeeds
  **When** it runs
  **Then** the event is `done`, with two `attempt` transitions, and the third run sees `DECREE_FINAL_ATTEMPT=true`

- **Given** a retry loop whose `cond` is `visits.implement < 2`
  **When** it runs
  **Then** `implement` is visited exactly twice, and `attempt` transitions are not counted

- **Given** a ScriptedRouter with a valid reply; an invalid event then a valid one; two invalid replies
  **When** a router state runs
  **Then** the event is taken with `source: llm`; the second reply is taken; `default` is taken with `router_error`

- **Given** `cond`s that leave one option
  **When** a router state runs
  **Then** `source` is `single_option` and the ScriptedRouter queue is untouched

- **Given** runs covering section 7
  **When** their events are read
  **Then** every `transition`, `script`, `router` and `run_finished` field appears in at least one test
