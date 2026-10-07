---
machine: rust_develop
---
# 92: A small project example replaces `feature`

## Overview

`examples/feature` is a frozen decree project that the reference docs quote and tests replay. Its `feature` machine uses every building block at once (nesting, a check, a model's choice, a person, `data`, `emits`, `onentry` and `onexit`), and six more machines sit around it. That teaches the opposite of the simplest-machine rule (migration 90). The user chose to replace it with a small one.

## Requirements

Read migrations 88, 90 and 91, the skill, `examples/feature/`, and every doc and test that uses it, first.

1. **`examples/project/`** replaces `examples/feature/` (`git rm -r` the old one). It is a frozen decree project, as `feature` was, and as small as it can be while showing what a project's files look like:
   - **Machines**, each the simplest that does its job, in the skill's style:
     - `hello`: one script;
     - `develop`: `implement` with `attempts: [local, claude]`, then `test`. The end of the skill's growth path.
     - `deploy`: `build`, a `person` approval, `ship`.
   - **Runs**, recorded the way `feature`'s were (same tooling, deterministic ids, timestamps and trace ids):
     - a finished `develop` migration whose `local` attempt fails and whose `claude` attempt succeeds, so the log shows a `source: "attempt"` transition with `attempt_value`;
     - a `deploy` run waiting for a person, and a queued reply to it in `inbox/`;
     - one interrupted run.
   - `processed.md`, one cron file, the graphs and the schemas.
   - Its scripts stand in for an AI agent and a deploy, as `feature`'s do.
   - A README in the style of the other examples, short: what it shows, the files, and the read-only commands.
2. **The docs** that quote `feature` (`docs/reference/runs.md`, `machines.md`, `graph.md`, `README.md`, `docs/routers.md`, `examples/observability/`, the main README) quote `project` instead. Where a doc used `feature` to show something `project` no longer has (a nested state, a `check`, a `model` decision and its router child run, `emits`, `onentry`/`onexit`), the doc shows a short fragment of its own. `tests/docs_api_test.rs` checks such fragments as whole machines.
3. **Tests.** Every test that uses `examples/feature` either uses `examples/project`, or, when it tests decree's behaviour with something only `feature` had (nesting, router child runs, `emits`, the graph of a compound state), keeps that coverage with a fixture under `tests/fixtures/`. No behaviour loses its test. List each change in your reply.
4. **`src/`** comments and tests that name `examples/feature` are updated.
5. **CHANGELOG.md**, under Changed: `examples/project`, three simple machines, replaces `examples/feature`.

- Only this migration's scope. No change to decree's behaviour.
- Do not edit `.decree/migrations/` or `.decree/runs/` at the repository root.
- If a doc cannot keep what it teaches without `feature`, write the question to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- Print the evidence for each acceptance criterion at the end of your reply.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass.

## Acceptance Criteria

- **Given** `examples/project`
  **When** `decree check` and `decree graph` run there
  **Then** both exit 0, `decree graph` changes nothing, and its recorded runs replay through decree to the same events

- **Given** its machines
  **When** they are read
  **Then** there are three (`hello`, `develop`, `deploy`), none has a compound state, a `check`, a `model` decision or a child machine, and `develop` uses `attempts: [local, claude]`

- **Given** the repository
  **When** `rg -n 'examples/feature|feature/README' --glob '!.decree/migrations/**' --glob '!.decree/runs/**' --glob '!CHANGELOG.md'` runs
  **Then** nothing matches

- **Given** the test suite
  **When** it runs
  **Then** it passes, and every behaviour a `feature`-based test covered is still covered, as listed in the reply
