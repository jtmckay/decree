---
machine: develop
---
# A11a: A reply sent before the deadline wins over the timeout, and a late reply never holds the queue

## Overview

A run in a `person` state with a `timeout` can receive a reply and a timeout at about the same time. Today the reply loses even when it was sent first, and when it loses it stops the whole project. The A12 run (the `lease` plugin) found and reproduced this. Leases make it routine: a client releasing a lease in the last second before it expires is normal.

1. A run waits in a `person` state with `timeout: 1s`. A client runs `decree event <id> release` just before the deadline. `decree event` accepts it, because the run is `waiting`.
2. On its next pass the daemon delivers timeouts before it reads `inbox/` (`drain` in `src/commands/process.rs`: `deliver_timeouts`, then `continue_runs`, then `next_inbox`). The deadline has passed by then, so the run takes `error` and finishes.
3. The reply is then claimed for a finished run and rejected, as documented (messages.md, Replies, step 4): it becomes a failed `invalid_message` run. But decree writes that run's attention file with `machine: ''`, and decree's own attention reader rejects it ("invalid frontmatter: `machine` is not a non-empty string"). An unreadable attention file holds the queue, so no machine runs until a person deletes the file.

So there are two defects:
- **The order is wrong.** Which comes first should depend on when the reply was sent, not on the order of the daemon's checks. AWS Step Functions works this way: a `SendTaskSuccess` made before the task's timeout completes the task, even if the state machine has not processed it yet. decree's own D8 already calls the reply the callback and the wait id the task token. A reply's id is the time it was queued (messages.md, Frontmatter keys), so decree already knows when it was sent.
- **A rejected reply holds the queue.** It should only be visible, as messages.md says.

## Outcome

- **A reply queued at or before a wait's deadline is delivered, not the timeout.** When decree finds that a waiting run's `timeout_at` has passed, it first looks in `inbox/` for a reply to that wait (by wait id, or by run id meaning its current wait) whose id time is at or before `timeout_at`. If there is one, decree delivers that reply as usual (Replies, step 3) and does not deliver the timeout. If that reply fails a check (for example an unknown option), it is rejected as today, and the timeout is delivered on the same or the next pass.
- The time that counts is the reply's id, which is when it reached the queue: the `decree event` or `emit` time, or for a file written straight into `inbox/`, the time decree gave it an id on arrival. The docs say this, so a program that writes replies knows a reply must be queued, not just written somewhere, before the deadline.
- A reply whose id time is after the deadline loses to the timeout, as today.
- **A rejected reply never holds the queue.** The run of a rejected reply (unknown run, run not waiting, stale wait id, unknown option) gets an attention file that is valid against `attention.schema.json` and has `holds: false`, whatever the reason it was rejected. A rejection run has no machine of its own, so the file names the machine of the run it was addressed to when there is one, and the schema and reader allow `machine` to be absent when there is none (an unknown run id). Choose one shape, apply it in the writer, the reader, the schema and the docs, and say why in the decision.
- A reply rejected for any other reason still shows in `decree status` as a failed `invalid_message` run, as today, and `process` reports it without stopping (Lifecycle, "A failed inbox run does not stop `process`").
- Nothing else about replies or timeouts changes: wait ids, `decree event`'s refusal of a second queued reply, `received` events and `timed_out: true` stay as they are.

## Constraints

- Core change only. Leave the `person` name and every other name alone: a later migration (A11b) renames it. Do not touch `plugins/lease.md` or `tests/lease_test.rs` if they exist in the work tree; they belong to the A12 run, which continues after this one.
- No new frontmatter key and no new event type.
- Only this migration's scope.
- Never edit `.decree/migrations/`, `.decree/runs/` or `.decree/processed.md`, not even by a search and replace across the repository: exclude them from every bulk edit.
- If the reference docs and this migration disagree in a way you cannot settle, write the question to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- No test calls a real model or the network. No new dependencies. Tests set the clock or deadlines directly; no test sleeps longer than it needs to.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass, and `decree check` passes in every example directory.
- Documentation:
  - `docs/reference/messages.md`, Replies: step 5 (Timeout) says that a reply queued at or before the deadline wins, and which time counts; step 4 (Reject) says the rejection run's attention file never holds;
  - `docs/reference/cli.md`, Attention: the file of a rejected reply, and its `machine`;
  - the decree skill template, where it covers replies and timeouts;
  - CHANGELOG lists both under Fixed;
  - `docs/decisions.md` gets a decision citing Step Functions' callback pattern (a task token sent before the timeout completes the task) and saying why the reply's id time, not the delivery time, decides.

## Acceptance Criteria

- **Given** a run waiting in a `person` state whose deadline has passed, and a reply to its wait in `inbox/` with an id time before the deadline
  **When** `decree process` runs
  **Then** the run takes the reply's event with `source: "person"`, no `timed_out` event is written, and the reply is in `runs/<id>/received/`

- **Given** the same, but with a reply to the run id (not the wait id) queued before the deadline
  **When** `decree process` runs
  **Then** the reply is delivered, not the timeout

- **Given** a waiting run whose deadline has passed, and a reply with an id time after the deadline
  **When** `decree process` runs
  **Then** the run takes `error` with `timed_out: true`, the reply becomes a failed `invalid_message` run, and its attention file has `holds: false`

- **Given** a reply queued before the deadline that names an option the state does not have
  **When** `decree process` runs after the deadline
  **Then** the reply is rejected as `invalid_message`, and the run takes the timeout's `error`

- **Given** a reply to a run that has finished, and a reply to a run id that does not exist
  **When** `decree process` runs
  **Then** each becomes a failed `invalid_message` run whose attention file is valid against `attention.schema.json` and has `holds: false`; `decree status` lists them and nothing holds; a migration queued behind them still runs

- **Given** a run waiting with `timeout: 1s`, and `decree event <id> <option>` run before the deadline
  **When** the next `decree daemon` pass starts only after the deadline
  **Then** the reply's event is taken, and the daemon does not stop on a holding attention file
