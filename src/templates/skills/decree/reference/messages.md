# Messages

A message is one markdown file: YAML frontmatter (structured) plus a body (the task, free text).
One message is one run. decree reads and writes only the frontmatter and never changes the body.

```markdown
---
machine: feature
params:
  max_rounds: 3
---
# Add rate limiting to /api/upload
Given ... When ... Then ...
```

## Frontmatter keys

| Key | Who sets it | Meaning |
| --- | --- | --- |
| `machine` | Author, `decree emit`, cron | Machine name (`machines/<name>.yml`). Required: there is no default machine. |
| `params` | Author, `decree emit --param` | Values for the machine's `data` (string, int, number or bool), one of the `enum` values if the entry has one. Unknown names, wrong types and values outside the `enum` fail validation. |
| `id` | decree, at claim | `YYYYMMDDTHHMMSS.ffffffZ-xxxxxx`, UTC to the microsecond, so ids sort in queue order; a migration's id is its file stem. Names the run folder. |
| `state` | decree only | Mirror of the run's current state, in `runs/<id>/message.md`. Never set it. |
| `parent`, `depth` | `decree emit`, decree | The run that emitted this message (or invoked this child run), and the chain depth. |
| `trigger` | decree | `migration`, `cron`, `emit`, `inbox` or `invoke`. |
| `traceparent`, `tracestate` | decree, `decree emit`, any tool | W3C Trace Context: the run joins this trace, under this span. An invalid `traceparent` is ignored and the run starts a new trace. |
| `to`, `event` | `decree event`, any tool | Only in a reply to a waiting run (below). |

Any other key is kept as written and ignored.

## Migrations: ordered, run once

`.decree/migrations/*.md` is the queue for work that must happen exactly once, in order, with its
history in git.

- **Name** them with a numeric prefix: `01-add-login.md`, `02-rate-limit-upload.md`. They run in
  byte order of filename. Use the next free number.
- **Immutable.** decree never edits, moves or renames a migration; neither do you. A run works on
  a copy in `runs/<stem>/message.md`. To fix a processed migration, write a new one that says
  what it fixes.
- **Ledger.** `.decree/processed.md` (committed) lists one filename per line. When a migration's
  run reaches a final state other than `failed`, decree appends its filename *before* that final
  state's `onentry` scripts run, so `done: { final: true, onentry: [commit] }` commits the code
  and the ledger line together. decree never runs git itself.
- **Strict order.** Migration N+1 starts only when N is processed and `inbox/` is empty, so
  follow-ups that N emits run first. A migration whose run is `failed`, `interrupted` or
  `waiting` blocks every later one, and a failed migration stops `decree process` at once.
  A failed follow-up has left `inbox/`, so it does not hold back the next migration.
- **Validated first.** `decree process` checks every pending migration (frontmatter, machine,
  `params`) before running any, and runs nothing if one is invalid.

### Writing a good migration

- One concern, day-sized: implementable in one agent session and reviewable on its own. Five
  subsystems means five migrations.
- Self-contained: no dependency on a sibling migration's runtime state.
- Acceptance criteria as Given / When / Then, each automatable:

  ```markdown
  - **Given** <precondition>
    **When** <action>
    **Then** <observable result: exit code, file contents, output, response>
  ```

  Avoid outcomes like "works correctly".

## Inbox: first in, first out

`.decree/inbox/*.md` holds queued messages, run in filename order. Not committed.

- A failed inbox run does not stop `decree process`: it prints the run with
  `decree process --retry <id>` and goes on with the next file. When the inbox is empty it lists
  every run that failed in the pass and exits 1.

- People may drop a file in directly. Programs write `.<name>.tmp`, then rename it to `<name>.md`;
  files starting with `.` are ignored, so nobody reads a partial file.
- From a script, use `decree emit`, which does the temp-file-and-rename for you, sets `parent`,
  `depth` and `trigger: emit`, and refuses past `max_depth` (10):

  ```bash
  decree emit --machine feature --param max_rounds=3 <<'EOF'
  # Follow-up: split rate limiting per route
  Given ... When ... Then ...
  EOF
  ```

- Inside a run, the emitting state must list the target machine in its `emits:`; otherwise
  `decree emit` exits 1. `decree graph` draws these edges in `system.md`.

## Cron

`.decree/cron/*.md` files are message templates that `decree daemon` drops into `inbox/` on a
schedule. The `cron:` key is a standard cron expression; the rest is an ordinary message.

```markdown
---
cron: "0 3 * * *"
machine: develop
---
Run `cargo audit` and update any dependency that has a patched version.
```

`decree status --cron` lists them with their next fire time.

## Replies to a waiting run

A run in a `person` state waits for a reply with a **wait id** (`<run id>.w<seq>`). The
state's `ask` script tells someone the question, the options and how to answer. A reply is an
inbox message with `to:` (the wait id, or the run id meaning "its current wait") and `event:`
(one of the options); the body is an optional note the run's later scripts read through
`DECREE_RECEIVED`.

```bash
decree event 02-upload-quota-per-plan.w15 retry -m "plans.toml lives in config/."
```

writes:

```markdown
---
id: 20261001T160301Z-9be210
to: 02-upload-quota-per-plan.w15
event: retry
---
plans.toml lives in config/.
```

Any tool (a chat bot, a web UI) can write the same file. A reply to a stale wait id or with an
event that is not an option is not applied; it shows up in `decree status` as a failed message.
