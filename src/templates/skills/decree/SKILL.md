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
  `error`, or a JSON last stdout line names a richer event (`{"event":"pass"}`).

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
- **Read `.decree/schema/machine.schema.json` before writing a machine**, and write against it.
  It is the JSON Schema of every machine key: types, required keys, each `invoke` kind and
  condition, name patterns, with a description and examples for each. Every machine starts
  with `# yaml-language-server: $schema=../schema/machine.schema.json`, so editors check it
  too. Run `decree schema` if `.decree/schema/` is missing.
- **Machines decide, scripts work.** A script makes no routing decision beyond printing one
  event; a decision is a state of its own (`check`, `model` or `person`), never logic hidden in a script.
- **Queue follow-up work with `decree emit`**, never by writing into `.decree/inbox/` by hand
  from a script. The emitting state must list the target in `emits:`.
- **Run `decree check` after every change** to a machine, script, message or cron file, and
  `decree graph` after changing a machine. Commit `.decree/graph/`.
- **Scripts must be safe to re-run**: `decree retry` re-runs a step that was interrupted.
- Do not commit `.decree/inbox/` or `.decree/runs/`.

## Commands

| Command | Use |
| --- | --- |
| `decree check` | Validate every machine, script name, pending migration, inbox message and cron file. Exit 1 lists one error per line. |
| `decree graph` | Write `.decree/graph/<machine>.md` (Mermaid) for every machine, plus `system.md`. |
| `decree schema` | Write the JSON Schemas for machines and message frontmatter to `.decree/schema/`. |
| `decree emit --machine <m> [--param k=v]...` | Queue a message for machine `m`; the body comes from stdin. Prints the new id. |
| `decree process [--dry-run]` | Run everything queued: replies, pending runs, the inbox (FIFO), then migrations in order. |
| `decree daemon [--interval <s>]` | The same, in a loop, with cron. |
| `decree status [<id>] [--cron]` | Runs by status; one run's events; cron schedule. |
| `decree tail [<id>]` | Follow the output of the script running now. |
| `decree event <wait id> <event> [-m <note>]` | Answer a run waiting in a `person` state. |
| `decree retry <id> [--state <s>]` | Continue an interrupted (or finished) run. |
| `decree prune --older-than <age> [--dry-run]` | Delete finished run folders older than `30d`, `12h`, `90m`; keeps failed migrations and children of unfinished runs. |

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
# yaml-language-server: $schema=../schema/machine.schema.json
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
  runs, `decree retry`, graphs.

The reference files sit next to this file, in `reference/`.
