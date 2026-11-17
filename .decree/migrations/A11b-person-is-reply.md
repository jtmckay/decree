---
machine: develop
---
# A11b: The `person` invoke is `reply`, and its `ask` script is `notify`

## Overview

The `person` invoke pauses a run until a reply message picks one of the state's transitions. decree has never known who replies (D40: "decree never knows it is a question"). The `lease` plugin (A12) uses the same invoke for a program: a lease client releases or renews by `decree event`, and the "ask" script tells the client that it holds the lease. Nothing in that is a person or a question. Calling the invoke `person` makes it look as if only people can answer, which keeps it from being used for callbacks from programs.

The user wants it named for what it does. decree already calls the message that answers it a **reply** (`to:`, `event:`, `decree event`, messages.md Replies, "waits for a reply"). So the invoke becomes `reply`. This is the same mechanism under its own name: the AWS Step Functions callback pattern, with the wait id as the task token (D8), and BPMN's receive task, a step that waits for a message from an outside participant, person or system. The script that tells someone a reply is wanted becomes `notify`: it may tell a person, a UI or a program, and it does not always ask a question.

## Outcome

- In a machine, `invoke: { reply: { question, notify, timeout? } }` replaces `invoke: { person: { question, ask, timeout? } }`. Everything else about it is unchanged: the options are the state's transitions with their descriptions, the wait id, the reply message, the timeout and A11a's rule that a reply queued before the deadline wins.
- The renamed names, everywhere decree writes or reads them:
  - the SCXML invoke type `decree:person` is `decree:reply`;
  - the `source` of a transition taken from a reply is `reply` (events schema and runs.md);
  - the graph edge label `(person)` is `(reply)`, and the graph note `person: <ask script>` is `reply: <notify script>`;
  - every error, warning, SARIF rule text and `--format json` field that names the invoke or its `ask` script (V8, V12, V16, V18, the Invoke table);
  - the JSON schemas in `schema/v1/` (`machine`, `events`, `attention`, `cli/status` and any other that names it).
- A machine that still says `person:`, or `ask:` inside it, fails `decree check` under V19, as every replaced shape does, with the one to write instead: ``person: is now reply: (and its ask: is notify:)``. There is no alias: the old spelling never runs.
- The official plugins (`plugins/*.md`), the examples (`examples/`, including their recorded runs, graphs and an `ask_person` script, renamed for example to `notify_person`), the README, the docs and the decree skill template use the new names, and `decree check` passes in every example.
- **Only the invoke is renamed, not the word "person".** Prose that means a human stays as it is: "only a person continues an interrupted run", "a person answers it in plan.md", "worth asking a person". Prose about the invoke that assumed a human ("a person picks one option") now says someone or something replies, and the example `deploy` machine, which really does ask a person, still says so in its descriptions.
- Script environment names are unchanged (`DECREE_WAIT_ID`, `DECREE_QUESTION`, `DECREE_CHOICES`, `DECREE_RECEIVED`): none of them names a person.

## Constraints

- A rename only: no change in behaviour. Every test that passed still passes, with the new names in its fixtures and expected output.
- This changes the `v1` file contract in a way that is not additive. decree is 0.x, so this is allowed, and CHANGELOG lists it under Changed with what to rewrite (`person:` to `reply:`, `ask:` to `notify:`, `source: "person"` to `source: "reply"` for tools that read events).
- No compatibility alias in the code; the V19 message is the only place the old name stays. Existing decisions in `docs/decisions.md` keep their text, because they record what was decided at the time. The new decision says what replaced them.
- If `plugins/lease.md` or `tests/lease_test.rs` exist in the work tree (left by the A12 run, which continues after this migration), rename their `person:` and `ask:` too, so they still pass `decree check`. Make no other change to them.
- Only this migration's scope.
- Never edit `.decree/migrations/`, `.decree/runs/` or `.decree/processed.md`, not even by a search and replace across the repository: exclude them from every bulk edit. The installed copies in `.decree/plugins/` come from `plugins/`; update them the way A10 does (`decree plugin install`), not by hand.
- If the reference docs and this migration disagree in a way you cannot settle, write the question to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- No test calls a real model or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass, and `decree check` passes in every example directory.
- Documentation:
  - `docs/reference/machines.md`, `messages.md`, `runs.md`, `graph.md`, `cli.md`, `scripts.md`, `standards.md`, `docs/escalation.md`, `docs/routers.md` and the decree skill template use `reply` and `notify`;
  - `docs/reference/standards.md` (or the Invoke table) cites BPMN's receive task and the Step Functions callback pattern for `reply`;
  - CHANGELOG lists the rename under Changed;
  - `docs/decisions.md` gets a decision: why `reply` (decree's own word for the answering message; BPMN receive task; Step Functions callback) and not `receive`, `callback` or `external`, why `notify` and not `ask`, and that D8, D40 and the other `person` decisions now read as `reply`.

## Acceptance Criteria

- **Given** a machine with `invoke: { reply: { question: …, notify: tell, timeout: 1h } }` and two described options
  **When** `decree check`, `decree graph` and `decree process` run, and a reply is sent with `decree event`
  **Then** check passes, the graph labels the edges `(reply)`, the run waits with the wait id, and the reply's transition has `source: "reply"`

- **Given** a machine that still uses `person:` with `ask:`
  **When** `decree check` runs
  **Then** it fails under V19, and the message says to write `reply:` with `notify:`

- **Given** the repository after the change
  **When** searching outside `.decree/migrations/`, `.decree/runs/`, `.decree/processed.md`, `docs/decisions.md`'s earlier entries, CHANGELOG's earlier entries and the V19 message
  **Then** no `person:` invoke key, `decree:person`, `"person"` source, `(person)` label or `ask:` key of that invoke remains

- **Given** every example directory and every official plugin
  **When** `decree check` runs
  **Then** it passes, and the regenerated graphs show `(reply)`

- **Given** `decree schema`
  **When** the schemas are written
  **Then** `machine.schema.json` accepts `reply` with `notify` and rejects `person`, and `events.schema.json` lists `reply` as a `source`
