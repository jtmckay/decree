# Messages

A message is one markdown file. decree reads and writes only the frontmatter, and never changes the body.

```markdown
---
id: 20261001T143005.218734Z-3fa9c1
machine: feature
state: verify
parent: 20261001T120000.051907Z-0b12aa
depth: 1
trigger: emit
params:
  max_rounds: 3
---
# Add rate limiting to /api/upload

## Overview
## Outcome
## Constraints
## Acceptance Criteria
```

## Writing a message

decree never reads the body: a message with any headings is valid and runs the same. The body is a spec for whoever does the work, usually an agent, and these rules keep it one.

- **One concern per message**, day-sized: implementable in one agent session and reviewable on its own.
- **Name an order file so it keeps its order.** Order files run in byte order of filename, so `100-` sorts before `99-`. Pad the number to the width the project will need (`001-add-login.md`); a project that outgrows it continues with a letter prefix (`A01-`, then `B01-`), because letters sort after digits ([Order](#order-files-run-in-order-at-each-start)).
- **Describe the outcome, not the steps.** A message says what is true when it is done and why; the agent chooses how. A message that says *how* ties the agent to the implementation its author pictured, and when the code differs, the run stops or the agent forces the plan. This is declarative configuration, as `infra` declares a desired state and `apply` reaches it, like Ansible and Terraform; the acceptance criteria are BDD's Given / When / Then.
  - **Outcome** (belongs in a message): behaviour a user or another program can observe; the public contract (command and flag names, file names, formats and locations, exit codes, messages printed); what must not change; why the change is wanted.
  - **Steps** (left out): which functions, modules or files to change, in what order, which data structures. One is named only as a constraint ("no new dependencies", "decree itself runs no git command"), never as a suggestion.
  - The test for any sentence: if a different correct implementation would fail it, it is a step.
- **The body** has four sections: `## Overview` (why), `## Outcome` (what is true afterwards), `## Constraints` (what must hold or not change) and `## Acceptance Criteria` (Given / When / Then, each observable: exit code, file contents, output, response).

One sentence, written as a step and as an outcome:

> **Step:** The warning has its own rule id, following the existing warnings.
>
> **Outcome:** `decree check` shows this warning and still exits 0, in text, JSON and SARIF.

The step assumed the existing warnings had rule ids; they had none, and the run stopped. The outcome holds for any correct implementation.

## Frontmatter keys

| Key | Type | Required | Written by | Meaning |
| --- | --- | --- | --- | --- |
| `id` | string | No | decree only: `decree emit`, `decree reply` and cron as they queue, decree as a file arrives in `inbox/` | `YYYYMMDDTHHMMSS.ffffffZ-xxxxxx`: UTC time to the microsecond plus 6 lowercase hex chars. Names the queued file and the run folder; a child run gets one as it starts. |
| `machine` | string | Yes | Author, `decree emit`, cron | Machine name, matching `machines/<name>/machine.yml`. A cron file or order file without one runs its folder's machine. |
| `state` | string | No | decree only | Mirror of the run's current state, in `runs/<id>/message.md` only. Authors never set it. |
| `parent` | string | No | `decree emit`, decree | `id` of the run that emitted this message, or that invoked this child run. |
| `depth` | int | No | `decree emit` | Parent's `depth` + 1. Absent means 0. |
| `trigger` | string | No | decree only | `cron`, `emit`, `inbox` or `invoke` (a child run, as an order file's run is). Exposed as `DECREE_TRIGGER`. |
| `arrived_as` | string | No | decree only | The name a file written into `inbox/` had when it arrived, before decree gave it its id ([Lifecycle](#lifecycle) step 1), so a person can find the file they wrote. Kept in the queued file and in `runs/<id>/message.md`. |
| `params` | map of string to string, int, number or bool | No | Author, `decree emit` | Sets the machine's `data` for this run, overriding defaults (like SCXML `<invoke><param>`). Each value must be of its entry's `type` (an int is a valid `number`) and, if the entry has an `enum`, one of its values. Unknown names, wrong types and values outside the `enum` fail validation, naming the param, the value and the allowed values. |
| `traceparent` | string | No | decree, `decree emit`, any tool | W3C Trace Context `00-<trace id>-<parent span id>-<flags>`: the run joins that trace, its run span a child of that span. decree writes it for child runs, `decree emit` from the script's `TRACEPARENT`, and any tool may set it, such as a webhook relaying an upstream trace. An invalid one is ignored and the run starts a new trace ([Traces](observability.md#traces)). |
| `tracestate` | string | No | decree, `decree emit`, any tool | W3C Trace Context vendor data, passed on unchanged beside a valid `traceparent`: to every script as `TRACESTATE`, to child runs and to emitted messages. |
| `to` | string | Reply messages only | Author, `decree reply`, any tool | The wait id (or run id) of a run paused in a `reply` state. Makes the message a reply, not a new run ([Replies](#replies)). |
| `event` | string | Reply messages only | Author, `decree reply`, any tool | The event to deliver. |

`.decree/schema/v1/message.schema.json` states these keys as a JSON Schema, with the two shapes a frontmatter takes: a message that names its `machine`, or a reply with `to` and `event` ([Schemas](README.md#schemas)).

`id`, `state`, `trigger` and `arrived_as` are decree's alone: in a file placed in `inbox/` they are replaced by decree's own as it arrives, so a copy of any message (an order file, a run's `message.md`) runs again as a new run. Any other key is kept exactly as written and ignored by decree; the `order` plugin's `order:` is one.

The time is ISO 8601 basic format with a decimal fraction of the second. Ids sort in the order they were made, so messages queued one after another run in that order, even within one second: one process never makes two ids with the same or an earlier time, even if the clock steps back, as RFC 9562 (section 6.2) has UUIDv7 generators do. The 6 hex chars are the low 24 bits of (sub-second nanoseconds XOR process id), and keep apart ids that processes make at the same microsecond; such messages were queued at once, so either order is right. If that id already exists in `inbox/` or `runs/`, decree adds 1 to them and retries.

## Parsing and writing

- A UTF-8 BOM at the start is ignored. Lines may end in `\n` or `\r\n`, mixed.
- Frontmatter exists only if line 1, with trailing whitespace removed, is `---`. It ends at the next line that, with trailing whitespace removed, is `---`. No opening fence means an empty map and the whole file is the body.
- An opening fence with no closing fence is a parse error. It is never treated as body.
- Unknown keys and key order survive a read and a write. Duplicate keys and a non-mapping top level are parse errors. Parse errors name the file and the file line (not the line inside the YAML).
- decree writes back `---\n` + the serialized mapping + `---\n` + the original body bytes, unchanged (including their line endings).
- Every write by decree goes to a temp file `.<name>.tmp` in the same directory, then a rename over the target. Nothing ever writes a message in place.

## Lifecycle

1. **Queue.** A file appears in `inbox/`. Humans may write directly. Programs must write `.<name>.tmp` then rename; `decree emit`, `decree reply` and cron do this for you and name the file `<id>.md`, with that `id` in its frontmatter. Files whose names start with `.` are ignored, so a writer can write `.x.md` and rename it.
   **Arrival.** Every other file gets an id the first time `daemon` sees it, as Maildir names a message on delivery, so the order never depends on what the writer called it: a file not named `<id>.md` by an id decree made, one whose frontmatter `id` is not that id, and one whose id already has a run (a copy of a run's `message.md`). Its id is a fresh one from that moment, so it runs after everything already queued; files first seen together get theirs in order of modification time, then name. decree drops the file's `id`, `state`, `trigger` and `arrived_as`, sets `id` first and `arrived_as` to the file's name, keeps every other key and the body, and renames it to `inbox/<id>.md`. A file whose frontmatter does not parse is renamed unchanged, and its run fails at claim (step 3). From then on it is an ordinary queued message. One process does this at a time, holding `inbox/.lock` as a [run lock](#run-lock) is held; a file a run is being claimed from is never renamed.
2. **Claim.** decree takes the queued files in byte-order of filename, their ids, so each machine's messages start first-in, first-out in the order they arrived. A file with `to:` is a reply and is delivered instead ([Replies](#replies)). Otherwise decree sets `trigger: inbox` if missing, creates `runs/<id>/`, and renames the file to `runs/<id>/message.md`. The rename is the claim: if it fails with not-found, another process won, so decree skips it.
3. **Validate.** If the frontmatter does not parse, `machine` is unknown, or a param is unknown or the wrong type, the run starts in `failed`: decree writes one `transition` event with `to: "failed"`, `source: "invalid_message"` and the reason, mirrors `state: failed` (unless the frontmatter itself did not parse, then `message.md` is left unchanged), appends `run_finished` with `state: "failed"` and `duration_ms: 0`, and runs nothing.
4. **Run.** The interpreter steps the machine ([runs.md](runs.md)). A run in a `reply` state pauses until a reply arrives ([Replies](#replies)).
5. **Finish.** The run ends when it enters a root-level final state. The run folder is kept until the official [`prune`](plugins.md#official-plugins) plugin removes it, weekly, once it is older than `PRUNE_OLDER_THAN`; decree itself removes none.
6. **Interrupt.** A run that stops before a final state is *interrupted*, and decree never continues it on its own. Stopping is often deliberate, and decree cannot tell a deliberate kill from a crash, so it does not guess. Only a person continues an interrupted run, with `decree daemon --once --retry` ([Retry](cli.md#retry)).

A run that ends in `failed`, is interrupted or waits for a person has a file in `.decree/attention/` saying what happened and the command that continues it, until its status changes ([Attention](cli.md#attention)).

**A failed inbox run does not stop `daemon --once`.** Inbox messages are independent, so when a run claimed from `inbox/` ends in `failed` (a reply's rejection run, or a run continued after a reply, included), `decree daemon --once` reports it with `decree daemon --once --retry <id>` and goes on with the next inbox file. Once the queue is empty it lists every run that failed in the pass and exits 1.

**Source of truth.** The run's state is the `to` of the last `transition` event in `events.jsonl`. `message.md`'s `state` is a mirror for humans and tools; whenever decree touches a run and the two disagree (a crash between the two writes), it rewrites the mirror. This is event sourcing: the log is the record, everything else is derived.

**Run status** is derived from the last event, in this order:

| Status | Condition |
| --- | --- |
| `finished` | The last event is `run_finished`, or the last `transition` event's `to` is a final state. |
| `active` | The run's `.lock` holds a live pid. |
| `waiting` | The last event is `waiting`: the run asked for a reply (`reply`), or is waiting for a child run. |
| `pending` | The last event is `received`, or a `transition` with `source: "retry"`. `daemon` continues it. |
| `interrupted` | Anything else. Includes a run whose last event is `interrupted`, and a run left mid-step by a crash. |

A run that reaches a root final state appends `run_finished` last ([runs.md](runs.md#step-loop), step 8), and only `decree daemon --once --retry` appends after it. So a run whose last event is `run_finished` is finished, and when `daemon`, `decree status` or `decree tail` look over every run, decree reads only that last line of its `events.jsonl`: a finished run costs one small read, however long its log. `decree status <id>` and stepping a run still read the whole log.

When `daemon` starts, it appends an `interrupted` event with `cause: "crash"` to every run that is interrupted but whose last event is not already `interrupted`, so the stop is visible in the log. It continues `pending` runs, in `id` order, before reading `inbox/`.

## Replies

A `reply` invoke ([machines.md](machines.md#invoke-the-states-function)) pauses the run until a reply picks an option. decree does not know who replies or how they were told: the `notify` script tells a person, a UI or a program, and the reply may come from any of them. This is SCXML's external event queue, BPMN's receive task (a step that waits for a message from an outside participant), and the same pattern as the AWS Step Functions callback (`.waitForTaskToken`): the wait id plays the task token.

1. **Wait.** decree runs the `notify` script with the **wait id** `<run id>.w<seq>` (`seq` is that of the `transition` event that entered the state) and the options in its environment ([scripts.md](scripts.md#environment)), so it can tell a person, a UI or another system how to reply. Then it appends a `waiting` event with the wait id, the options and the deadline if `timeout` is set, releases the lock, and the run's status becomes `waiting`.
2. **Reply.** A reply is an ordinary inbox message with `to:` (the wait id, or the run id meaning "its current wait") and `event:` (one of the options). The body is optional: a note or data for the run's later scripts. `decree reply <wait id> <event> [-m <note>]` writes one through the `emit` writer; any program can write one the same way ([Lifecycle](#lifecycle) step 1). A reply written under any other name gets an id as it arrives, as every message does, so replies are delivered in the order they arrived. `decree reply` refuses a reply while another reply to the same wait is still queued in `inbox/`, since only the first can be delivered; a reply written directly is checked only on delivery (step 4).
3. **Deliver.** When decree claims a reply, it checks that the run is `waiting`, that `to` is the current wait id (or the run id), and that `event` is one of the options. Then it moves the message to `runs/<run id>/received/<filename>`, appends a `received` event, and the run is `pending`: decree continues it at once, taking the transition for that event with `source: "reply"`.
4. **Reject.** A reply that fails any check (unknown run, run not waiting, stale wait id, unknown option) becomes a run of its own that ends `failed` with `invalid_message` and the reason, so it is visible in `decree status`. The waiting run is unchanged. Like any failed inbox run, it does not stop `daemon --once` ([Lifecycle](#lifecycle)), and its file in `.decree/attention/` never holds the queue (`holds: false`): it names the machine of the run the reply was addressed to, and has no `machine` when that run does not exist ([Attention](cli.md#attention)).
5. **Timeout.** If the invoke sets `timeout` ([Durations](machines.md#durations)), every `daemon` pass checks the deadline. Once it has passed, decree first looks in `inbox/` for a reply to that wait (its wait id, or the run id) queued at or before the deadline, and delivers it as in step 3, in the order they were queued, until one is delivered: a reply sent in time wins even when decree gets to it after the deadline, as a Step Functions task token sent before the task's timeout completes the task ([D81](../decisions.md#d81-a-reply-queued-before-the-deadline-wins)). One that fails a check is rejected as in step 4. When none is delivered, decree appends a `received` event for `error` with `timed_out: true`, and continues the run. The time that counts is the reply's id, the time it reached the queue ([Frontmatter keys](#frontmatter-keys)): when `decree reply` or `decree emit` queued it, or, for a file written straight into `inbox/`, when decree gave it an id on arrival ([Lifecycle](#lifecycle) step 1). So a program that writes replies must queue a reply, not just write it somewhere, before the deadline; a reply whose id is later than the deadline loses to the timeout, and is then rejected because the run is no longer waiting.

`decree daemon --once` ends by printing every waiting run, its question (the state's `description`), its options and the `decree reply` command for each, and exits 0. The same is in the run's file in `.decree/attention/` until the reply is delivered ([Attention](cli.md#attention)); the file is read-only, so a reply always goes through `decree reply` or a reply message.

**Stopping.** On SIGINT or SIGTERM, decree stops the running script ([scripts.md](scripts.md#execution)), appends an `interrupted` event with `cause: "signal"`, deletes the lock and exits. No `onexit` scripts run; `decree daemon --once --retry` re-runs the `onentry` scripts instead ([runs.md](runs.md#step-loop), step 1).

## Run lock

Before stepping a run, decree creates `runs/<id>/.lock` exclusively (`O_EXCL`) and writes the process id into it. If it already exists, decree reads the pid. If `kill(pid, 0)` says that process is alive, the run is `active`: skip it. Otherwise the lock is stale: the run was interrupted by a crash ([Lifecycle](#lifecycle)). decree deletes the lock when the run reaches a final state or is interrupted by a signal. The lock guards against two processes; it never causes a takeover.

## Order files: run in order at each start

Work that must happen in order, and once (setting a project up, a series of changes), is a machine's order files: `.decree/machines/<machine>/order/*.md`, committed with the project. decree itself runs no order: the official [`order`](plugins.md#official-plugins) plugin, which `decree init` adds, does, at each daemon start (`decree daemon` and `decree daemon --once`) ([D86](../decisions.md#d86-machines-own-their-folders-order-replaces-migrations)). An order file says how something should be, not a step to repeat, so editing it runs it again.

1. **Order.** Files run one at a time, across every machine's `order/`, in byte order of file name, the machine's name breaking a tie.
2. **Everything before it first.** A file starts only once `inbox/` is empty, so the follow-ups the file before it emitted have run. A follow-up that ends in `failed` has left `inbox/`, so it holds nothing back.
3. **A child run.** The file runs as a child run of the `order` run (`invoke: { machine: { message } }`, [machines.md](machines.md#invoke-the-states-function)): its `machine` is its folder's unless it names another, and it gets `order: <machine>/<file>`, by which a script (the official `commit`) knows which file it is.
4. **Once, and again when it changes.** A file that finishes in any state but `failed` is recorded, `<file> <sha256>`, in its machine's `state/order.git.txt`, which git keeps. A file whose hash is recorded does not run again; an edited file does.
5. **Stop on error.** A file whose run fails stops the files after it: the `order` run fails, with its file in `.decree/attention/`, and the next start runs that file again. `decree daemon --once --retry <id>` continues the failed run, as for any run.

**Batching by resource.** Rule 2 batches work by resource for free. An order file that does the LLM work and emits a message for each GPU step runs every LLM step first; only then do its emitted messages run, all the ComfyUI work, before the next file starts. Ollama and ComfyUI then swap the GPU once per file, not once per step.

**Committing.** decree never runs git. The official `commit` script records the file in `order.git.txt` before it commits, so the commit holds the work and the record, and a fresh clone knows which files ran.

## Run a message again

To run a message again (an order file, a run's `message.md`, any saved message), queue a copy as a new inbox run:

```bash
decree emit < .decree/machines/develop/order/01-a.md
```

or copy the file into `inbox/`. Either way the copy keeps its frontmatter and body, and gets decree's own `id`, `state` and `trigger` ([Lifecycle](#lifecycle) step 1), so it is a new run: the order file and `order.git.txt` are untouched. `decree emit --machine <m>` runs it on another machine, and `--param` overrides one param ([decree emit](cli.md)).

## Cron files

`.decree/machines/<machine>/cron/*.md` are message templates with a `cron:` key in their frontmatter, a standard cron expression. On each `decree daemon` pass, every cron file that is due is written to `inbox/` through the same writer as `decree emit`: the new message has the cron file's `machine`, `trigger: cron`, the cron file's other keys and its body; the `cron` key is not copied. A cron file must name its machine. decree reads `cron/*.md` and, one folder down, `cron/<folder>/*.md`, where a plugin installs its cron files ([plugins.md](plugins.md)); a folder deeper is not read, nor a file whose name starts with `.`. `decree check` validates every cron file (M3, [Validation](machines.md#validation)), and `decree status --cron` lists them with their next fire time, by path (`<file>.md`, `<plugin>/<file>.md`).

`cron: "@reboot"` fires once each time `decree daemon` starts, at its first pass, as Vixie cron's `@reboot` fires when the cron daemon starts, not when the host boots ([D80](../decisions.md#d80-reboot-fires-when-the-daemon-starts)). For a daemon that runs from boot to shutdown, that is on boot: warm a model, check that services answer, catch up on what was missed while the host was down.

```markdown
---
cron: "@reboot"
machine: boot
---
Check that the services this project calls answer.
```

The message goes through the same writer with `trigger: cron`, and `--machine` and `--exclude-machine` decide whether this daemon fires it, as for any cron file. It fires on every start, `decree daemon --retry` included, so a daemon a supervisor restarts after a crash queues it again each time; decree keeps no record of past starts. A script that must not repeat its work checks its own result first, as every script should. It fires only at start: a `@reboot` file added while the daemon runs waits for the next start, and `decree daemon --once` never fires it, as it fires no cron file. When a file in `.decree/attention/` holds the queue (or after `decree stop`), the message is still queued at start and runs once the hold is gone. `decree status --cron` shows `daemon start` in place of its next fire time; `--format json` gives it `next: null` and `at_daemon_start: true`.
