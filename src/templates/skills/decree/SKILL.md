---
name: decree
description: >
  Work in a decree project: messages (migrations, inbox, cron), machines (YAML statecharts in
  .decree/machines/) and scripts (.decree/scripts/), checked with `decree check`, drawn with
  `decree graph`, and queued with `decree emit`.
  INVOKE when: the user mentions decree or .decree/; writes or edits a migration, inbox message,
  cron file, machine or script; asks how to automate, schedule or chain work, ask a model or a
  person to decide a step, or why a run failed, is waiting or was interrupted.
  SKIP for: shell scripting, CI or infrastructure work unrelated to decree.
---

# decree

decree runs work through three building blocks:

- A **message** (markdown) says *what* to do in its body, and *which machine* does it
  (`machine:` in the frontmatter). Migrations, inbox messages and cron templates are all messages.
- A **machine** (`.decree/machines/<name>.yml`) says *in what order*: a W3C SCXML statechart
  written in YAML. States, what each state invokes, and which event leads to which state. No
  code, no paths.
- A **script** (`.decree/scripts/<name>.sh`, or `.decree/scripts/<machine>/<name>.sh` for one
  machine's own) does one piece of work and reports one outcome: exit 0 is `done`, non-zero is
  `error`, or the script names a richer event in a file (`echo pass > "$DECREE_EVENT_FILE"`).

Every state invokes one function, and its result is an event. The function is a script, a child
machine (`machine: deploy`), or a built-in decision: `check` (deterministic: `true` or `false`),
`model` (a model picks an option, asked through a router machine) or `person` (the run pauses
until a reply picks one). `invoke` has exactly one key, which names the kind. AI and people
appear only where a machine invokes `model` or `person`.

## Rules

- **Never edit a migration.** Files in `.decree/migrations/` are immutable; the ones listed in
  `.decree/processed.md` have run. To change something, write a new migration with the next
  number.
- **One concern per migration**, day-sized, with Given / When / Then acceptance criteria whose
  outcomes are observable (exit codes, file contents, output).
- **Always set `machine:`** in a message. Pick from `.decree/machines/` (`ls .decree/machines`).
- **Read `.decree/schema/v1/machine.schema.json` before writing a machine**, and write against it.
  It is the JSON Schema of every machine key: types, required keys, each `invoke` kind and
  condition, name patterns, with a description and examples for each. Every machine starts
  with `# yaml-language-server: $schema=../schema/v1/machine.schema.json`, so editors check it
  too. Run `decree schema` if `.decree/schema/v1/` is missing. The same folder holds the
  schemas of `events.jsonl` lines (`events.schema.json`) and of a router's `request.json` and
  `reply.json`: read them before writing a router or anything that reads a run.
- **Machines decide, scripts work.** A script makes no routing decision beyond printing one
  event; a decision is a state of its own (`check`, `model` or `person`), never logic hidden in a script.
- **Route with a typed router.** For a `model` decision that routes work (which model, which
  path), point `router:` at a typed router (a classifier such as GLiNER2.5-Decide, or a model
  constrained by `reply_schema`): it cannot answer outside the options and costs little. Keep
  untyped models (`claude -p`) for the work itself.
- **Queue follow-up work with `decree emit`**, never by writing into `.decree/inbox/` by hand
  from a script. The emitting state must list the target in `emits:`.
- **Run `decree check` after every change** to a machine, script, message or cron file, and
  `decree graph` after changing a machine. Commit `.decree/graph/`.
- **Read decree's output as JSON, never by parsing its text**: `decree check --format json`,
  `decree status [<id>] --format json`, and `--format json` on `emit`, `event`, `prune`,
  `graph`, `schema` and `process --dry-run` print one JSON document, described by
  `.decree/schema/v1/cli/<command>.schema.json`. Exit codes are the same as in text.
  `decree check --format json` gives each error's `rule`, `file`, `line` or `state` and
  `message`.
- **Scripts must be safe to re-run**: `decree process --retry` re-runs a step that was interrupted.
- Do not commit `.decree/inbox/` or `.decree/runs/`.

## Commands

| Command | Use |
| --- | --- |
| `decree check [--format json\|sarif]` | Validate every machine, script name, pending migration, inbox message and cron file. Exit 1 lists one error per line (`json`: one document; `sarif`: a SARIF 2.1.0 log for code scanning). |
| `decree graph` | Write `.decree/graph/<machine>.md` (Mermaid) for every machine, plus `system.md`. |
| `decree schema` | Write the JSON Schemas of machines, messages, events and router files to `.decree/schema/v1/`. |
| `decree emit --machine <m> [--param k=v]...` | Queue a message for machine `m`; the body comes from stdin. Prints the new id. |
| `decree process [--dry-run]` | Run everything queued: replies, pending runs, the inbox (FIFO), then migrations in order. |
| `decree process --retry [<id>] [--state <s>]` | Continue a failed or interrupted run first: `<id>`, or the migration blocking the queue. The failure message prints the exact command. |
| `decree daemon [--interval <duration>]` | The same, in a loop, with cron, every `2s` by default. |
| `decree status [<id>] [--cron] [--format json]` | Runs by status; one run's events; cron schedule (text only). |
| `decree tail [<id>]` | Follow the output of the script running now. |
| `decree event <wait id> <event> [-m <note>]` | Answer a run waiting in a `person` state. |
| `decree prune --older-than <age> [--dry-run]` | Delete finished run folders older than `30d`, `12h`, `90m`, `90s`; keeps failed migrations and children of unfinished runs. |

## Worked example

A migration for the `feature` machine:

```markdown
---
machine: feature
---
# Rate-limit /api/upload

## Requirements

Limit each API key to 10 uploads per minute on `POST /api/upload`.

## Acceptance Criteria

- **Given** a key that has uploaded 10 times in the last minute
  **When** it uploads again
  **Then** the response is 429 with `Retry-After`
```

The smallest machine:

```yaml
# yaml-language-server: $schema=../schema/v1/machine.schema.json
# Graph: ../graph/hello.md
name: hello
description: Run one script.
initial: greet
states:
  greet:
    invoke: greet                  # runs scripts/greet (or scripts/hello/greet)
    transitions: { done: done }    # exit 0 -> done; non-zero -> failed, implicitly
  done:   { final: true }
  failed: { final: true }          # every machine has one
```

## Reference files

Read the one you need; don't load them all upfront:

- **`reference/messages.md`**: frontmatter keys, migrations and `processed.md`, the inbox,
  cron files, `decree emit`, replies to a waiting run.
- **`reference/machines.md`**: the schema line, every machine key, the invoke types, conditions, choices and
  routers, composition, events and transitions, validation rules, full examples.
- **`reference/scripts.md`**: where scripts live, how they run, how they report an event,
  every `DECREE_*` variable.
- **`reference/runs.md`**: run folders, `events.jsonl`, run status, waiting and interrupted
  runs, `decree process --retry`, graphs.

The reference files sit next to this file, in `reference/`.
