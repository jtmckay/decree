---
machine: develop
---
# 101: Say plainly when an attempt list moves on, and when to use a child machine instead

## Overview

From real use: a machine had `attempts: [local, claude]`, and the local model wrote `STOP` (the script named `stop`). The user expected Claude to answer the local model's question. It did not: a named event ends the attempts, and only `error` moves to the next value. The rule is in `scripts.md` ("the first attempt that does not end in `error` decides the event"), but not where people read about attempt lists.

## Requirements

1. **`docs/reference/machines.md`** (where `attempts` is described) and the skill's `reference/machines.md`: say plainly, in two or three sentences, that the next attempt runs only after `error` (a non-zero exit or a timeout); `done` or any event the script names, such as `stop`, ends the list at once and goes to the state's transitions.
2. **When a stronger tier should answer a weaker tier's question** (its `STOP`), that is not an attempt: make it a transition to a child machine. Show a short fragment:
   ```yaml
   implement:
     invoke:
       script: { name: implement, attempts: [local, local] }
     transitions: { done: gate, stop: escalate }
   escalate:                        # a stronger model answers the local model's question
     invoke:
       machine: { name: escalate }
     transitions: { answered: implement, failed: failed }
   ```
   and say what the child reads (`$DECREE_PARENT_RUN_DIR/STOP`) and writes back (the answer, for the next `implement` visit). Note that a child machine's final states are transitions every caller must handle, so keep them few.
3. **The skill's SKILL.md** gets one line under the attempts guidance: "`attempts` retries after `error` only; to have a stronger model answer a weaker one's `STOP`, transition to a child machine."
4. `tests/docs_api_test.rs` checks the fragment as a whole machine, as it does others. CHANGELOG under Changed (docs).

- Only this migration's scope.
- Never edit `.decree/migrations/`, `.decree/runs/` or `.decree/processed.md`, not even by a search and replace across the repository: exclude them from every bulk edit.
- If the reference docs and this migration disagree in a way you cannot settle, write the question to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- No test calls a real model or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass, and `decree check` passes in every example directory.

## Acceptance Criteria

- **Given** `docs/reference/machines.md` and the skill
  **When** they are read
  **Then** each says that only `error` moves an attempt list on, and shows the child-machine pattern for answering a `STOP`

- **Given** `attempts: [local, claude]` and a script that names `stop` on its first attempt (an existing or new test)
  **When** it runs
  **Then** the state takes `stop`, and the script does not run with `claude`
