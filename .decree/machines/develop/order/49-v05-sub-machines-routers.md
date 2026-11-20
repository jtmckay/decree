---
routine: rust-develop
---
# 49: v0.5 M3.3 Sub-machines and router machines

## Overview

Run child machines: `invoke: { machine: … }`, and `choose: model` through a replaceable router machine. Part of the 0.5.0 rewrite specified in `docs/0.5-spec.md` (ticket M3.3).

## Requirements

Read spec sections 1 and 3 (config.yml), 5 (Invoke), 6 (Environment), 7 (Step loop, Choose: model, The default router, Sub-machines, events.jsonl); `mock/`; `docs/spikes/router.md` (decision record) first.

Implement child runs as section 7 describes (own `runs/<id>/`, `parent`, `depth`, `trigger: invoke`, parent `waiting`, `received` or `decision` when the child finishes, children paused on a person). Implement `choose: model`: write `request.json`, run the router machine (`router` or `default_router`) with `DECREE_REQUEST` and `DECREE_REPLY`, validate `reply.json`, apply `min_confidence`. Replace `commands` in `config.yml` with `default_router`. Make `decree init` write `claude_router` and its `ask_claude` script (as in `mock/`, moving 0.4.2's `invoke_ai_router` and its reply parsing into that script, then deleting it and its tests; until M5.3 a 0.4 message with no `routine:` goes to `default_routine`). `--ai copilot|opencode` writes `copilot_router`/`opencode_router`: the same machine with an `ask_copilot`/`ask_opencode` script that differs only in the CLI call. Write `docs/routers.md` with router machines for: TypeSafe Jev (the request maps to one Choice question: `question` to `instructions`, options to `criteria`, input and message to `state`; `choice`, `probabilities` and `confidence` map straight back); Fastino's GLiNER2.5-Decide run locally (a small long-running server so the 1B model loads once, not per decision; options as described labels, `include_confidence`); OpenAI's Decisions API, marked as pending until its schema is published; and a cheap-model-first escalation. Explain that `min_confidence` is calibrated per router. Running a model server is out of scope.

- Only this migration's scope; the other v0.5 migrations cover the rest of spec section 11.
- If the code does something the spec does not cover, or the spec is ambiguous here, do not guess: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes. The routine fails the run when `STOP` exists.
- Tests build their own `.decree/` in a temp directory and never touch this repository's `.decree/`. No test calls a real LLM or the network. No new dependencies unless the spec names them.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass. The routine checks this itself after you finish.

## Files to Modify

- src/interpreter.rs
- src/config.rs
- src/commands/init.rs
- src/templates/
- docs/routers.md — new
- tests/fixtures/

## Acceptance Criteria

- **Given** a `machine` invoke
  **When** it runs
  **Then** the child has its own `runs/<id>/` with `parent`, `depth` and `trigger: invoke`, and the parent takes the child's final state as its event (`failed` as `error`)

- **Given** a child that reaches `choose: person`
  **When** it runs
  **Then** the parent stays `waiting`; the reply finishes the child, then the parent

- **Given** a test router machine whose script copies `$DECREE_REQUEST` and writes `{"event":"retry","confidence":0.9}` to `$DECREE_REPLY`
  **When** a `choose: model` state runs
  **Then** the parent takes `retry`, and the copied request matches section 7

- **Given** a reply with confidence 0.5 and `min_confidence: 0.8`
  **When** it is read
  **Then** the event is `unsure` and the `decision` event records the pick

- **Given** a reply naming no option, or a router run that ends `failed`
  **When** it is read
  **Then** the event is `error` with `router_error`

- **Given** a stub `claude` on `PATH` that prints prose then a fenced JSON object
  **When** `ask_claude` runs
  **Then** it writes the right `reply.json`

- **Given** an empty directory
  **When** `decree init --ai claude` runs
  **Then** `machines/claude_router.yml` exists, `default_router: claude_router` is set, and `decree check` passes
