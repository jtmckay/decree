---
routine: rust-develop
---
# 65: v0.5 cleanup: reliability tests

## Overview

Make the tests prove that decree behaves as documented end to end, through the binary, so they survive any refactor of `src/` (migration 66). Three kinds: the mock's runs replayed as acceptance tests, failure scenarios, and property tests of the interpreter's invariants.

## Requirements

Read `docs/reference/runs.md` (step loop, `events.jsonl`), `docs/reference/scripts.md`, `docs/reference/messages.md` (lifecycle, replies, run lock), `docs/reference/cli.md`, `mock/README.md` and `tests/README.md` first.

1. **Mock replay** (`tests/mock_replay_test.rs`). For each run in `mock/.decree/runs/` that is not a router child run: copy `mock/.decree` to a temp project, replace every script with a stub that reproduces, execution by execution, the exit code and output recorded in that run's `script` events and logs, and replace each router script with one that writes the run's recorded `reply.json`. Queue the run's message, run `decree process` (and, for runs that received a reply, deliver it with `decree event` and run `decree process` again). Compare the produced `events.jsonl` with the mock's after normalising timestamps, durations, `started_at`, `timeout_at` and generated child run ids. The interrupted run is replayed by sending SIGINT to `decree process` while its script runs. If a mock run cannot be reproduced because the mock and the documented behaviour disagree, do not change either: write the difference to a file named `STOP` in the run directory and end.
2. **Failure scenarios** through the binary, each in its own test, adding only those `tests/` does not cover yet (say which existed):
   - SIGTERM during a script: the run is `interrupted` (cause `signal`), the script's process group is gone, and `decree retry` continues it and reruns the script;
   - `decree process` killed with SIGKILL: the next `decree process` records `interrupted` (cause `crash`) from the stale lock, naming the script from `.running`;
   - two `decree process` at once: the second skips the active run;
   - a router whose reply is not an option, twice: the state's event is `error` with `router_error`;
   - `min_confidence` above the reported confidence: `unsure`; a missing confidence: `unsure`;
   - a reply with a stale wait id, and one with an event that is not an option: the run keeps waiting, and the reply ends as a failed `invalid_message` run;
   - `timeout_s` on `choose: person` passing: `error`;
   - a child run past `max_depth`: `error` without a child;
   - an `onexit` failure: recorded in `exit_failures`, the target unchanged;
   - a log over `max_log_size`: truncated as documented;
   - a migration that ends in `failed`: not added to `processed.md`, and later migrations do not run.
3. **Property tests** (`tests/interpreter_props.rs`, with `proptest` as a dev-dependency). Generate small valid machines (up to 6 states, at most one level of compound states, script and `check` invokes, finals at both levels) and random script outcomes (exit codes, printed events, declared or not). For every case: `decree check` passes on the machine; the run ends in a root-level final state; `seq` in `events.jsonl` is 1, 2, 3, … with no gaps; every `transition`'s `to` is a state of the machine and its `from` is the previous transition's `to`; `visits` derived from the events equals the number of transitions into each state; the `state:` mirror in `message.md` equals the last transition's `to`; and `onentry`/`onexit` scripts run in the documented order. Keep the default case count so `cargo test` stays under a minute.
4. Update `tests/README.md` with the new files.

- Only this migration's scope; migration 66 covers the refactor.
- Change no behaviour. If a test shows the binary disagreeing with the reference docs, do not change the binary or weaken the test: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end. The routine fails the run when `STOP` exists.
- Tests build their own `.decree/` in a temp directory and never touch this repository's `.decree/`. No test calls a real LLM or the network. The only new dependency is `proptest`, as a dev-dependency.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass. The routine checks this itself after you finish.

## Files to Modify

- tests/mock_replay_test.rs — new
- tests/ — failure scenarios
- tests/interpreter_props.rs — new
- Cargo.toml — `proptest` dev-dependency
- tests/README.md

## Acceptance Criteria

- **Given** each non-router run in `mock/.decree/runs/`
  **When** `tests/mock_replay_test.rs` runs
  **Then** the produced events equal the mock's after normalisation

- **Given** each failure scenario above
  **When** `cargo test` runs
  **Then** a test with a name that says the scenario passes (print the names)

- **Given** `tests/interpreter_props.rs`
  **When** `cargo test` runs
  **Then** every property holds, and the whole suite finishes in under a minute (print the time)
