# Messages

A message is one markdown file. decree reads and writes only the frontmatter, and never changes the body.

```markdown
---
id: 20261001T143005Z-3fa9c1
machine: feature
state: verify
parent: 20261001T120000Z-0b12aa
depth: 1
trigger: emit
params:
  max_rounds: 3
---
# Add rate limiting to /api/upload
Given ... When ... Then ...
```

## Frontmatter keys

| Key | Type | Required | Written by | Meaning |
| --- | --- | --- | --- | --- |
| `id` | string | No | decree, at claim, if missing | `YYYYMMDDTHHMMSSZ-xxxxxx`: UTC time plus 6 lowercase hex chars. Names the run folder. (Migrations: the file stem.) |
| `machine` | string | Only if `default_machine` is unset | Author, `decree emit`, cron | Machine name, matching `machines/<name>.yml`. |
| `state` | string | No | decree only | Mirror of the run's current state, in `runs/<id>/message.md` only. Authors never set it. |
| `parent` | string | No | `decree emit`, decree | `id` of the run that emitted this message, or that invoked this child run. |
| `depth` | int | No | `decree emit` | Parent's `depth` + 1. Absent means 0. |
| `trigger` | string | No | decree | `migration`, `cron`, `emit`, `inbox` or `invoke` (a child run). Exposed as `DECREE_TRIGGER`. |
| `params` | map of string to string, int or bool | No | Author, `decree emit` | Sets the machine's `data` for this run, overriding defaults (like SCXML `<invoke><param>`). Unknown names fail validation. |
| `to` | string | Reply messages only | Author, `decree event`, any tool | The wait id (or run id) of a run paused in a `choose: person` state. Makes the message a reply, not a new run ([Replies](#replies)). |
| `event` | string | Reply messages only | Author, `decree event`, any tool | The event to deliver. |

`routine` is read as an alias of `machine`: migration files are immutable, and existing projects have unprocessed migrations that still carry it. Any other key is kept exactly as written and ignored by decree.

The 6 hex chars are the low 24 bits of (sub-second nanoseconds XOR process id). If that id already exists in `inbox/` or `runs/`, decree adds 1 and retries.

## Parsing and writing

- A UTF-8 BOM at the start is ignored. Lines may end in `\n` or `\r\n`, mixed.
- Frontmatter exists only if line 1, with trailing whitespace removed, is `---`. It ends at the next line that, with trailing whitespace removed, is `---`. No opening fence means an empty map and the whole file is the body.
- An opening fence with no closing fence is a parse error. It is never treated as body.
- Unknown keys and key order survive a read and a write. Duplicate keys and a non-mapping top level are parse errors. Parse errors name the file and the file line (not the line inside the YAML).
- decree writes back `---\n` + the serialized mapping + `---\n` + the original body bytes, unchanged (including their line endings).
- Every write by decree goes to a temp file `.<name>.tmp` in the same directory, then a rename over the target. Nothing ever writes a message in place.

## Lifecycle

1. **Queue.** A file appears in `inbox/`. Humans may write directly. Programs must write `.<name>.tmp` then rename; `decree emit` and cron do this for you and name the file `<id>.md`. Files whose names start with `.` are ignored.
2. **Claim.** decree takes the inbox file with the lowest byte-order filename, so emitted and cron messages run first-in, first-out. A file with `to:` is a reply and is delivered instead ([Replies](#replies)). Otherwise decree assigns `id` if missing, sets `trigger: inbox` if missing, creates `runs/<id>/`, and renames the file to `runs/<id>/message.md`. The rename is the claim: if it fails with not-found, another process won, so decree skips it.
3. **Validate.** If the frontmatter does not parse, `machine` is unknown, or a param is unknown or the wrong type, the run starts in `failed`: decree writes one `transition` event with `to: "failed"`, `source: "invalid_message"` and the reason, mirrors `state: failed` (unless the frontmatter itself did not parse, then `message.md` is left unchanged), and runs nothing.
4. **Run.** The interpreter steps the machine ([runs.md](runs.md)). A run in a `choose: person` state pauses until a reply arrives ([Replies](#replies)).
5. **Finish.** The run ends when it enters a root-level final state. The run folder is kept. Cleanup is out of scope.
6. **Interrupt.** A run that stops before a final state is *interrupted*, and decree never continues it on its own. Stopping is often deliberate, and decree cannot tell a deliberate kill from a crash, so it does not guess. Only `decree retry <id>` continues an interrupted run ([cli.md](cli.md)).

**Source of truth.** The run's state is the `to` of the last `transition` event in `events.jsonl`. `message.md`'s `state` is a mirror for humans and tools; whenever decree touches a run and the two disagree (a crash between the two writes), it rewrites the mirror. This is event sourcing: the log is the record, everything else is derived.

**Run status** is derived from the last event, in this order:

| Status | Condition |
| --- | --- |
| `finished` | The last `transition` event's `to` is a final state. |
| `active` | The run's `.lock` holds a live pid. |
| `waiting` | The last event is `waiting`: the run asked for a reply (`choose: person`), or is waiting for a child run. |
| `pending` | The last event is `received`, or a `transition` with `source: "retry"`. `process` and `daemon` continue it. |
| `interrupted` | Anything else. Includes a run whose last event is `interrupted`, and a run left mid-step by a crash. |

When `process` or `daemon` starts, it appends an `interrupted` event with `cause: "crash"` to every run that is interrupted but whose last event is not already `interrupted`, so the stop is visible in the log. It continues `pending` runs, in `id` order, before reading `inbox/`.

## Replies

A `choose: person` invoke ([machines.md](machines.md#invoke-the-states-function)) pauses the run until a person picks an option. decree does not know who is asked or how: the `ask` script does that. This is SCXML's external event queue, and the same pattern as the AWS Step Functions callback (`.waitForTaskToken`): the wait id plays the task token.

1. **Wait.** decree runs the `ask` script with the **wait id** `<run id>.w<seq>` (`seq` is that of the `transition` event that entered the state) and the options in its environment ([scripts.md](scripts.md#environment)), so it can tell a person, a UI or another system how to reply. Then it appends a `waiting` event with the wait id, the options and the deadline if `timeout_s` is set, releases the lock, and the run's status becomes `waiting`.
2. **Reply.** A reply is an ordinary inbox message with `to:` (the wait id, or the run id meaning "its current wait") and `event:` (one of the options). The body is optional: a note or data for the run's later scripts. `decree event <wait id> <event> [-m <note>]` writes one through the `emit` writer; any program can write one the same way ([Lifecycle](#lifecycle) step 1).
3. **Deliver.** When decree claims a reply, it checks that the run is `waiting`, that `to` is the current wait id (or the run id), and that `event` is one of the options. Then it moves the message to `runs/<run id>/received/<filename>`, appends a `received` event, and the run is `pending`: decree continues it at once, taking the transition for that event with `source: "person"`.
4. **Reject.** A reply that fails any check (unknown run, run not waiting, stale wait id, unknown option) becomes a run of its own that ends `failed` with `invalid_message` and the reason, so it is visible in `decree status`. The waiting run is unchanged.
5. **Timeout.** If the invoke sets `timeout_s`, every `process` and `daemon` pass checks the deadline. Once it has passed, decree appends a `received` event for `error` with `timed_out: true`, and continues the run.

A waiting migration blocks later migrations, as a `failed` or `interrupted` one does. `decree process` ends by printing every waiting run, its question (the state's `description`), its options and the `decree event` command for each, and exits 0.

**Stopping.** On SIGINT or SIGTERM, decree stops the running script ([scripts.md](scripts.md#execution)), appends an `interrupted` event with `cause: "signal"`, deletes the lock and exits. No `onexit` scripts run; `decree retry` re-runs the `onentry` scripts instead ([runs.md](runs.md#step-loop), step 1).

## Run lock

Before stepping a run, decree creates `runs/<id>/.lock` exclusively (`O_EXCL`) and writes the process id into it. If it already exists, decree reads the pid. If `kill(pid, 0)` says that process is alive, the run is `active`: skip it. Otherwise the lock is stale: the run was interrupted by a crash ([Lifecycle](#lifecycle)). decree deletes the lock when the run reaches a final state or is interrupted by a signal. The lock guards against two processes; it never causes a takeover.

## Migrations: ordered, run once, stop on error

`.decree/migrations/*.md` is a second queue for work that must happen exactly once, in order, with its history in git. Migrations are immutable: decree never edits, moves or renames them. The committed `processed.md` ledger records which have run. Any machine can run a migration; the queue only controls order.

1. **Run from a copy.** A migration's `id` is its file stem. To start one, decree creates `runs/<id>/` (already-exists means the migration has a run already: see rule 4) and copies the file to `runs/<id>/message.md`, adding `id` and `trigger: migration`. The run then works like any other.
2. **Run once.** A migration whose filename is in `processed.md` is skipped. `processed.md` is read as a set; duplicate lines are harmless.
3. **Strict order.** Migrations run in byte-order of filename. Migration N+1 starts only when N is in `processed.md` and `inbox/` is empty, so follow-ups that N emits run before N+1 starts.
4. **Stop on error.** A migration whose run is `failed` or `interrupted` blocks every later migration until `decree retry <id>` finishes it; `process` stops and exits 1, naming the migration and the `retry` command. A `waiting` migration also blocks later migrations until its reply arrives.
5. **Ledger write.** When a migration's run enters a final state other than `failed`, decree appends the filename to `processed.md` (temp file plus rename) before that state's `onentry` scripts run. If one of them fails ([runs.md](runs.md#step-loop)), decree removes the line again.
6. **Validate first.** Before starting the first pending migration, `process` and `daemon` parse every pending migration (frontmatter, machine, `params` against the machine's `data`). If any are invalid, they print every error and run nothing: `N migration(s) are invalid; nothing was processed.`, exit 1.

**Committing.** decree never runs git. Because the ledger is written before the final state's `onentry` scripts, put the commit there: `done: { final: true, onentry: [commit] }`. The commit then includes the code and the `processed.md` line, and a fresh clone knows which migrations ran. Do not commit `runs/`.

## Cron files

`.decree/cron/*.md` are message templates with a `cron:` key in their frontmatter, a standard cron expression. On each `decree daemon` tick, every cron file that is due is written to `inbox/` through the same writer as `decree emit`, with `trigger: cron`; the `cron` key is not copied. A cron file names its machine with `machine:` (or the `routine:` alias), else `default_machine` applies. `decree check` validates every cron file (M3, [Validation](machines.md#validation)), and `decree status --cron` lists them with their next fire time.
