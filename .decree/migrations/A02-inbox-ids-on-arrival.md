---
machine: develop
---
# A02: Every inbox message gets a time id, however it got there

## Overview

The inbox runs first-in, first-out by filename. `decree emit` names each file by its id (UTC time to the microsecond, D56), so emitted messages run in the order they were queued. A file a person or another tool writes into `inbox/` keeps whatever name it was given, so it runs before or after the rest by the accident of its name: `01-hi.md` jumps ahead of everything emitted, `zz.md` waits behind it. A copy of a run's `message.md` fails, because its `id:` already has a run. The user wants every inbox message treated as `emit` treats it. Prior art: Maildir, where a delivered message is given a unique, time-based name on arrival, so order never depends on what the sender called it.

## Outcome

- A message in `inbox/` runs in the order it arrived, whoever wrote it. A file whose name is not an id gets one the first time `process` or `daemon` sees it: a fresh id from that moment, like `emit`'s, and files first seen together are ordered by modification time, then name. From then on it is an ordinary queued message.
- The keys decree sets (`id`, `state`, `trigger`) are decree's alone: in a file placed in `inbox/` they are replaced by decree's own, so a copy of any message (a migration, a run's `message.md`) runs again as a new run. Every other frontmatter key and the body are kept.
- `decree status` lists such a file by its new name, and the run's `message.md` records the name it arrived with, so a person can find the file they wrote.
- Replies (`to:`) are ordered the same way.

## Constraints

- Files whose names start with `.` are still ignored, so a writer can write `.x.md` and rename it.
- A file is never claimed or renamed while another process holds it for a run (today's claim rules hold).
- Only this migration's scope.
- Never edit `.decree/migrations/`, `.decree/runs/` or `.decree/processed.md`, not even by a search and replace across the repository: exclude them from every bulk edit.
- If the reference docs and this migration disagree in a way you cannot settle, write the question to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- No test calls a real model or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass, and `decree check` passes in every example directory.
- CHANGELOG, the reference docs, the skill (and this repository's copy, as `decree skill` writes it) and a decision in `docs/decisions.md` say what changed and why.

## Acceptance Criteria

- **Given** an emitted message, then a file `01-hi.md` written into `inbox/` after it
  **When** `process` runs
  **Then** the emitted message runs first, then `01-hi.md` with a time id, and its `message.md` records `01-hi.md`

- **Given** `b.md` modified before `a.md`, both written while no process ran
  **When** `process` runs
  **Then** `b.md` runs first

- **Given** a copy of a finished run's `message.md` (with its `id:`) written into `inbox/`
  **When** `process` runs
  **Then** it runs as a new run with a new id, and the old run is unchanged

- **Given** `.draft.md` in `inbox/`
  **When** `process` runs
  **Then** it is neither renamed nor run
