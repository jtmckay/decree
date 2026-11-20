---
machine: develop
---
# A06: `max_depth` defaults to 10 and can be changed

## Overview

`max_depth` stops a chain of emits and child runs from looping forever. It is fixed at 10, so a project whose real chains go deeper (a router under a child machine under an emitted follow-up, several times over) cannot run them at all. The user wants 10 to be the default, not the limit. decree already reads two settings of its own from the shell or `.decree/.env`: `DECREE_MACHINES` and `DECREE_EXCLUDE_MACHINES` (108). This one is set the same way. Prior art: `DECREE_*` like `GIT_*` and `TF_*` variables, a recursion limit with a default that can be raised (bash's `FUNCNEST`, Python's `sys.setrecursionlimit`).

## Outcome

- `DECREE_MAX_DEPTH`, a whole number of at least 1, sets `max_depth` for the project. It can come from the shell or from `.decree/.env`, and the shell wins over `.decree/.env`. Without it, `max_depth` is 10, as now.
- The same limit applies everywhere a chain gets deeper: `decree emit` (from a person's shell or from a script inside a run), a `machine:` child run, and a `model` invoke's router run. Wherever it is set, `process`, `daemon` and an emit from inside their runs all use the same value.
- When the limit stops a chain, the message names the limit and how to change it. For example: `depth 11 exceeds max_depth 10 (set DECREE_MAX_DEPTH to change it)` from `decree emit`, and `max_depth 10 reached` in the events, as now.
- A value that is not a whole number of at least 1 is refused before anything runs. `process`, `daemon` and `emit` exit 1 naming the variable and its value, and `decree check` reports it as an error.
- `.decree/.env` may set `DECREE_MAX_DEPTH`. Every other `DECREE_*` name in it is still refused, and a machine's `env_file` still may not set any.
- The docs no longer call the depth fixed: the reference's "No configuration file" table, help, cli.md's environment table, scripts.md's Environment table, machines.md rule E1, and the skill's messages reference.

## Constraints

- Runs recorded under the old limit read and continue as before. Nothing about a run's folder, events or schemas changes, other than the limit's value in text that already shows it.
- No configuration file is added; this setting is a variable like the two before it.
- Only this migration's scope.
- Never edit `.decree/migrations/`, `.decree/runs/` or `.decree/processed.md`, not even by a search and replace across the repository: exclude them from every bulk edit.
- If the reference docs and this migration disagree in a way you cannot settle, write the question to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- No test calls a real model or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass, and `decree check` passes in every example directory.
- CHANGELOG under Changed, and a decision in `docs/decisions.md` saying why.

## Acceptance Criteria

- **Given** no `DECREE_MAX_DEPTH`
  **When** a chain reaches depth 11
  **Then** it is refused as today, and the message names `DECREE_MAX_DEPTH`

- **Given** `DECREE_MAX_DEPTH=20` in `.decree/.env`
  **When** a machine that emits to itself runs a chain 15 deep under `decree process`
  **Then** every emit is accepted and the run at depth 15 runs; at depth 21 the emit is refused

- **Given** `DECREE_MAX_DEPTH=2` in the shell and `20` in `.decree/.env`
  **When** a chain reaches depth 3, through a `machine:` child run or a `model` router run
  **Then** no child starts and the state's event is `error` with `max_depth 2 reached`

- **Given** `DECREE_MAX_DEPTH=0`, or `ten`
  **When** `decree check`, `decree process` and `decree emit` run
  **Then** each exits 1 naming `DECREE_MAX_DEPTH` and the value, and `process` runs nothing

- **Given** `DECREE_MAX_DEPTH` in a machine's `env_file`
  **When** `decree check` runs
  **Then** it fails as it does for any other `DECREE_*` name there
