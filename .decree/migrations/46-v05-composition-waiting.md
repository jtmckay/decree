---
routine: rust-develop
---
# 46: v0.5 M3.4 Composition, waiting states and SCXML conformance

## Overview

Add the rest of the SCXML subset to the interpreter: transitions on compound states with event bubbling, internal transitions, nested final states with `done.state.<id>`, and waiting states. Then check the interpreter against the W3C SCXML test suite where it applies. Part of the 0.5.0 rewrite specified in `docs/0.5-spec.md` (ticket M3.4).

## Requirements

Read spec sections 1 and 4 (Events for waiting runs), 5 (Kinds of state, SCXML subset, Rules), 7 (Step loop, events.jsonl: waiting and received) first.

Extend `src/interpreter.rs` as listed. For waiting states, implement only the interpreter side: entering one, the `waiting` event, and continuing from a `received` event; reply messages and `decree event` come with M4.3. Then survey the W3C SCXML 1.0 Implementation Report Plan (IRP) tests (https://www.w3.org/Voice/2013/scxml-irp/). If they cannot be downloaded, write that in `tests/fixtures/scxml/README.md` and port none; do not stop for this.

- Only this migration's scope; the other v0.5 migrations cover the rest of spec section 11.
- If the code does something the spec does not cover, or the spec is ambiguous here, do not guess: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes. The routine fails the run when `STOP` exists.
- Tests build their own `.decree/` in a temp directory and never touch this repository's `.decree/`. No test calls a real LLM or the network. No new dependencies unless the spec names them.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass. The routine checks this itself after you finish.

## Files to Modify

- src/interpreter.rs
- tests/fixtures/machines/
- tests/fixtures/scxml/ — new

## Acceptance Criteria

- **Given** an event an atomic state does not handle but its parent does
  **When** it is raised
  **Then** the parent's transition is taken

- **Given** a `type: internal` transition on a compound state to its child
  **When** it is taken
  **Then** the compound state's `onexit` and `onentry` do not run

- **Given** a compound state whose final child is entered
  **When** the run steps
  **Then** `done.state.<id>` is raised and handled at once with `source: internal`, and the run does not end

- **Given** a waiting state
  **When** it is entered
  **Then** its `onentry` sees `DECREE_WAIT_ID` and `DECREE_ACCEPTS`, a `waiting` event is appended, and stepping stops; a `received` event continues it without re-running any script

- **Given** the IRP tests, if downloadable
  **When** they are surveyed
  **Then** every test whose features all fall inside the section 5 subset is ported to a YAML fixture in `tests/fixtures/scxml/` and passes, and `tests/fixtures/scxml/README.md` lists every other test with the feature it needs
