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

- **One concern per migration**, day-sized: implementable in one agent session and reviewable on its own.
- **Name a migration so it keeps its order.** Migrations run in byte order of filename, so `100-` sorts before `99-`. Pad the number to the width the project will need (`001-add-login.md`); a project that outgrows it continues with a letter prefix (`A01-`, then `B01-`), because letters sort after digits. `decree check` warns when a new name sorts before one that already ran ([Migrations](#migrations-ordered-run-once-stop-on-error), rule 3).
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
| `id` | string | No | decree only: `decree emit`, `decree event` and cron as they queue, decree as a file arrives in `inbox/` | `YYYYMMDDTHHMMSS.ffffffZ-xxxxxx`: UTC time to the microsecond plus 6 lowercase hex chars. Names the queued file and the run folder. (Migrations: the file stem.) |
| `machine` | string | Yes | Author, `decree emit`, cron | Machine name, matching `machines/<name>.yml`. |
| `state` | string | No | decree only | Mirror of the run's current state, in `runs/<id>/message.md` only. Authors never set it. |
| `parent` | string | No | `decree emit`, decree | `id` of the run that emitted this message, or that invoked this child run. |
| `depth` | int | No | `decree emit` | Parent's `depth` + 1. Absent means 0. |
| `trigger` | string | No | decree only | `migration`, `cron`, `emit`, `inbox` or `invoke` (a child run). Exposed as `DECREE_TRIGGER`. |
| `arrived_as` | string | No | decree only | The name a file written into `inbox/` had when it arrived, before decree gave it its id ([Lifecycle](#lifecycle) step 1), so a person can find the file they wrote. Kept in the queued file and in `runs/<id>/message.md`. |
| `params` | map of string to string, int, number or bool | No | Author, `decree emit` | Sets the machine's `data` for this run, overriding defaults (like SCXML `<invoke><param>`). Each value must be of its entry's `type` (an int is a valid `number`) and, if the entry has an `enum`, one of its values. Unknown names, wrong types and values outside the `enum` fail validation, naming the param, the value and the allowed values. |
| `traceparent` | string | No | decree, `decree emit`, any tool | W3C Trace Context `00-<trace id>-<parent span id>-<flags>`: the run joins that trace, its run span a child of that span. decree writes it for child runs, `decree emit` from the script's `TRACEPARENT`, and any tool may set it, such as a webhook relaying an upstream trace. An invalid one is ignored and the run starts a new trace ([Traces](observability.md#traces)). |
| `tracestate` | string | No | decree, `decree emit`, any tool | W3C Trace Context vendor data, passed on unchanged beside a valid `traceparent`: to every script as `TRACESTATE`, to child runs and to emitted messages. |
| `to` | string | Reply messages only | Author, `decree event`, any tool | The wait id (or run id) of a run paused in a `person` state. Makes the message a reply, not a new run ([Replies](#replies)). |
| `event` | string | Reply messages only | Author, `decree event`, any tool | The event to deliver. |

`.decree/schema/v1/message.schema.json` states these keys as a JSON Schema, with the two shapes a frontmatter takes: a message that names its `machine`, or a reply with `to` and `event` ([Schemas](README.md#schemas)).

`id`, `state`, `trigger` and `arrived_as` are decree's alone: in a file placed in `inbox/` they are replaced by decree's own as it arrives, so a copy of any message (a migration, a run's `message.md`) runs again as a new run. Any other key is kept exactly as written and ignored by decree.

The time is ISO 8601 basic format with a decimal fraction of the second. Ids sort in the order they were made, so messages queued one after another run in that order, even within one second: one process never makes two ids with the same or an earlier time, even if the clock steps back, as RFC 9562 (section 6.2) has UUIDv7 generators do. The 6 hex chars are the low 24 bits of (sub-second nanoseconds XOR process id), and keep apart ids that processes make at the same microsecond; such messages were queued at once, so either order is right. If that id already exists in `inbox/` or `runs/`, decree adds 1 to them and retries.

## Parsing and writing

- A UTF-8 BOM at the start is ignored. Lines may end in `\n` or `\r\n`, mixed.
- Frontmatter exists only if line 1, with trailing whitespace removed, is `---`. It ends at the next line that, with trailing whitespace removed, is `---`. No opening fence means an empty map and the whole file is the body.
- An opening fence with no closing fence is a parse error. It is never treated as body.
- Unknown keys and key order survive a read and a write. Duplicate keys and a non-mapping top level are parse errors. Parse errors name the file and the file line (not the line inside the YAML).
- decree writes back `---\n` + the serialized mapping + `---\n` + the original body bytes, unchanged (including their line endings).
- Every write by decree goes to a temp file `.<name>.tmp` in the same directory, then a rename over the target. Nothing ever writes a message in place.

## Lifecycle

1. **Queue.** A file appears in `inbox/`. Humans may write directly. Programs must write `.<name>.tmp` then rename; `decree emit`, `decree event` and cron do this for you and name the file `<id>.md`, with that `id` in its frontmatter. Files whose names start with `.` are ignored, so a writer can write `.x.md` and rename it.
   **Arrival.** Every other file gets an id the first time `process` or `daemon` sees it, as Maildir names a message on delivery, so the order never depends on what the writer called it: a file not named `<id>.md` by an id decree made, one whose frontmatter `id` is not that id, and one whose id already has a run (a copy of a run's `message.md`). Its id is a fresh one from that moment, so it runs after everything already queued; files first seen together get theirs in order of modification time, then name. decree drops the file's `id`, `state`, `trigger` and `arrived_as`, sets `id` first and `arrived_as` to the file's name, keeps every other key and the body, and renames it to `inbox/<id>.md`. A file whose frontmatter does not parse is renamed unchanged, and its run fails at claim (step 3). From then on it is an ordinary queued message. One process does this at a time, holding `inbox/.lock` as a [run lock](#run-lock) is held; a file a run is being claimed from is never renamed.
2. **Claim.** decree takes the queued file with the lowest byte-order filename, its id, so messages run first-in, first-out in the order they arrived. A file with `to:` is a reply and is delivered instead ([Replies](#replies)). Otherwise decree sets `trigger: inbox` if missing, creates `runs/<id>/`, and renames the file to `runs/<id>/message.md`. The rename is the claim: if it fails with not-found, another process won, so decree skips it.
3. **Validate.** If the frontmatter does not parse, `machine` is unknown, or a param is unknown or the wrong type, the run starts in `failed`: decree writes one `transition` event with `to: "failed"`, `source: "invalid_message"` and the reason, mirrors `state: failed` (unless the frontmatter itself did not parse, then `message.md` is left unchanged), appends `run_finished` with `state: "failed"` and `duration_ms: 0`, and runs nothing.
4. **Run.** The interpreter steps the machine ([runs.md](runs.md)). A run in a `person` state pauses until a reply arrives ([Replies](#replies)).
5. **Finish.** The run ends when it enters a root-level final state. The run folder is kept until `decree prune` removes it ([cli.md](cli.md)); nothing removes it automatically.
6. **Interrupt.** A run that stops before a final state is *interrupted*, and decree never continues it on its own. Stopping is often deliberate, and decree cannot tell a deliberate kill from a crash, so it does not guess. Only a person continues an interrupted run, with `decree process --retry` ([Retry](cli.md#retry)).

A run that ends in `failed`, is interrupted or waits for a person has a file in `.decree/attention/` saying what happened and the command that continues it, until its status changes ([Attention](cli.md#attention)).

**A failed inbox run does not stop `process`.** Inbox messages are independent, so when a run claimed from `inbox/` ends in `failed` (a reply's rejection run, or a run continued after a reply, included), `decree process` reports it with `decree process --retry <id>` and goes on with the next inbox file. Once the queue is empty it lists every run that failed in the pass and exits 1. A failed migration does stop it ([Migrations](#migrations-ordered-run-once-stop-on-error), rule 4).

**Source of truth.** The run's state is the `to` of the last `transition` event in `events.jsonl`. `message.md`'s `state` is a mirror for humans and tools; whenever decree touches a run and the two disagree (a crash between the two writes), it rewrites the mirror. This is event sourcing: the log is the record, everything else is derived.

**Run status** is derived from the last event, in this order:

| Status | Condition |
| --- | --- |
| `finished` | The last event is `run_finished`, or the last `transition` event's `to` is a final state. |
| `active` | The run's `.lock` holds a live pid. |
| `waiting` | The last event is `waiting`: the run asked for a reply (`person`), or is waiting for a child run. |
| `pending` | The last event is `received`, or a `transition` with `source: "retry"`. `process` and `daemon` continue it. |
| `interrupted` | Anything else. Includes a run whose last event is `interrupted`, and a run left mid-step by a crash. |

A run that reaches a root final state appends `run_finished` last ([runs.md](runs.md#step-loop), step 8), and only `decree process --retry` appends after it. So a run whose last event is `run_finished` is finished, and when `process`, `daemon`, `decree status` or `decree tail` look over every run, decree reads only that last line of its `events.jsonl`: a finished run costs one small read, however long its log. `decree status <id>` and stepping a run still read the whole log.

When `process` or `daemon` starts, it appends an `interrupted` event with `cause: "crash"` to every run that is interrupted but whose last event is not already `interrupted`, so the stop is visible in the log. It continues `pending` runs, in `id` order, before reading `inbox/`.

## Replies

A `person` invoke ([machines.md](machines.md#invoke-the-states-function)) pauses the run until a person picks an option. decree does not know who is asked or how: the `ask` script does that. This is SCXML's external event queue, and the same pattern as the AWS Step Functions callback (`.waitForTaskToken`): the wait id plays the task token.

1. **Wait.** decree runs the `ask` script with the **wait id** `<run id>.w<seq>` (`seq` is that of the `transition` event that entered the state) and the options in its environment ([scripts.md](scripts.md#environment)), so it can tell a person, a UI or another system how to reply. Then it appends a `waiting` event with the wait id, the options and the deadline if `timeout` is set, releases the lock, and the run's status becomes `waiting`.
2. **Reply.** A reply is an ordinary inbox message with `to:` (the wait id, or the run id meaning "its current wait") and `event:` (one of the options). The body is optional: a note or data for the run's later scripts. `decree event <wait id> <event> [-m <note>]` writes one through the `emit` writer; any program can write one the same way ([Lifecycle](#lifecycle) step 1). A reply written under any other name gets an id as it arrives, as every message does, so replies are delivered in the order they arrived. `decree event` refuses a reply while another reply to the same wait is still queued in `inbox/`, since only the first can be delivered; a reply written directly is checked only on delivery (step 4).
3. **Deliver.** When decree claims a reply, it checks that the run is `waiting`, that `to` is the current wait id (or the run id), and that `event` is one of the options. Then it moves the message to `runs/<run id>/received/<filename>`, appends a `received` event, and the run is `pending`: decree continues it at once, taking the transition for that event with `source: "person"`.
4. **Reject.** A reply that fails any check (unknown run, run not waiting, stale wait id, unknown option) becomes a run of its own that ends `failed` with `invalid_message` and the reason, so it is visible in `decree status`. The waiting run is unchanged.
5. **Timeout.** If the invoke sets `timeout` ([Durations](machines.md#durations)), every `process` and `daemon` pass checks the deadline. Once it has passed, decree appends a `received` event for `error` with `timed_out: true`, and continues the run.

A waiting migration blocks later migrations, as a `failed` or `interrupted` one does. `decree process` ends by printing every waiting run, its question (the state's `description`), its options and the `decree event` command for each, and exits 0. The same is in the run's file in `.decree/attention/` until the reply is delivered ([Attention](cli.md#attention)); the file is read-only, so a reply always goes through `decree event` or a reply message.

**Stopping.** On SIGINT or SIGTERM, decree stops the running script ([scripts.md](scripts.md#execution)), appends an `interrupted` event with `cause: "signal"`, deletes the lock and exits. No `onexit` scripts run; `decree process --retry` re-runs the `onentry` scripts instead ([runs.md](runs.md#step-loop), step 1).

## Run lock

Before stepping a run, decree creates `runs/<id>/.lock` exclusively (`O_EXCL`) and writes the process id into it. If it already exists, decree reads the pid. If `kill(pid, 0)` says that process is alive, the run is `active`: skip it. Otherwise the lock is stale: the run was interrupted by a crash ([Lifecycle](#lifecycle)). decree deletes the lock when the run reaches a final state or is interrupted by a signal. The lock guards against two processes; it never causes a takeover.

## Migrations: ordered, run once, stop on error

`.decree/migrations/*.md` is a second queue for work that must happen exactly once, in order, with its history in git. Migrations are immutable: decree never edits, moves or renames them. The committed `processed.md` ledger records which have run. Any machine can run a migration; the queue only controls order.

1. **Run from a copy.** A migration's `id` is its file stem. To start one, decree creates `runs/<id>/` (already-exists means the migration has a run already: see rule 4) and copies the file to `runs/<id>/message.md`, adding `id` and `trigger: migration`. The run then works like any other.
2. **Run once.** A migration whose filename is in `processed.md` is skipped. `processed.md` is read as a set; duplicate lines are harmless.
3. **Strict order.** Migrations run in byte-order of filename. Migration N+1 starts only when N is in `processed.md` and `inbox/` is empty, so follow-ups that N emits run before N+1 starts. A follow-up that ends in `failed` has left `inbox/`, so it does not hold back N+1. A migration added with a name that sorts before one that already ran is allowed, and runs next, after the work that sorts later; `decree check` warns about each such pending migration, without failing: ``migration 02-b.md sorts before 03-c.md, which already ran; it runs next. To keep the order, pad numbers to the width the project needs (`001-`) or continue with a letter prefix (`A01-`), which sorts after digits``, where `03-c.md` is the last filename in `processed.md` in byte order.
4. **Stop on error.** A migration whose run is `failed` or `interrupted` blocks every later migration until `decree process --retry` continues it and it finishes; `process` stops at once and exits 1, unlike for a failed inbox run ([Lifecycle](#lifecycle)), naming the migration and ending ``Fix the cause, then run `decree process --retry`.`` A migration `waiting` for a child run that is `interrupted` (directly, or through children of its own) blocks the same way, naming the child: ``migration 03-api.md waits for child run <id>, interrupted in `<s>`; later migrations are blocked. Fix the cause, then run `decree process --retry`.``, and `--retry` continues that child; the migration's file in `.decree/attention/` names the child ([Attention](cli.md#attention)). Any other `waiting` migration also blocks later migrations until its reply arrives. When `process` ends with migrations still pending and none blocked, it prints why the first has not finished and exits 0: ``Held back: migration <file> waits for a reply: `decree event <wait id> <a|b>` ``, ``… waits for child run <id>, which waits for a reply: …``, ``… is active in another process (pid <pid>)``, followed by ``; <n> later migration(s) wait behind it``.
5. **Ledger write.** When a migration's run enters a final state other than `failed`, decree appends the filename to `processed.md` (temp file plus rename) before that state's `onentry` scripts run. If one of them fails ([runs.md](runs.md#step-loop)), decree removes the line again.
6. **Validate first.** Before starting the first pending migration, `process` and `daemon` parse every pending migration (frontmatter, machine, `params` against the machine's `data`). If any are invalid, they print every error and run nothing: `N migration(s) are invalid; nothing was processed.`, exit 1.

**Batching by resource.** Rule 3 batches work by resource for free. A migration that does the LLM work and emits a message for each GPU step runs every LLM step first; only then do its emitted messages run, all the ComfyUI work, before the next migration starts. Ollama and ComfyUI then swap the GPU once per migration, not once per step.

**Committing.** decree never runs git. Because the ledger is written before the final state's `onentry` scripts, put the commit there: `done: { final: true, onentry: [commit] }`. The commit then includes the code and the `processed.md` line, and a fresh clone knows which migrations ran. Do not commit `runs/`.

## Cron files

`.decree/cron/*.md` are message templates with a `cron:` key in their frontmatter, a standard cron expression. On each `decree daemon` pass, every cron file that is due is written to `inbox/` through the same writer as `decree emit`: the new message has the cron file's `machine`, `trigger: cron`, the cron file's other keys and its body; the `cron` key is not copied. A cron file must name its machine. `decree check` validates every cron file (M3, [Validation](machines.md#validation)), and `decree status --cron` lists them with their next fire time.
