---
name: decree
description: >
  Work in a decree project: messages (migrations, inbox, cron), machines (YAML statecharts in
  .decree/machines/) and scripts (.decree/scripts/), checked with `decree check`, drawn with
  `decree graph`, and queued with `decree emit`. Prefers the simplest machine: states are added
  only when the user asks or a run shows the need.
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

- **Start with the simplest machine.** Write the most naive machine that does the job: the
  scripts it needs, one state each, in a straight line (`done` → the next state), ending in
  `done`. `error` already goes to `failed` implicitly; don't write it.
  - Add nothing else until the user asks for it or a run has shown it is needed: no `check`,
    `model` or `person` decisions, loops or round counters, `data` params, `attempts`,
    `timeout`, child machines, `emits`, compound states or one state per case.
  - When an addition is warranted, add the smallest one that solves the problem, and say why
    in the state's comment (`# the local model fails often: try Claude second`).
  - The cheapest fixes come first, in this order: change the script; `attempts`; a transition
    on an event the script names; a `check`; a `model` or `person` decision; a child machine.
  - Asked to design a machine, show the simple version, and list possible additions in prose
    as options, not in the YAML.
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
- **Scripts choose how, machines choose what runs next.** A script may decide *how* to do its
  one job from its params and environment: which workflow file, which model for this attempt,
  which flags. A choice of *which step runs next* is a transition, on `done` or an event the
  script names. Make it a decision state (`check`, `model`, `person`) only when the step after
  it differs, and the choice is worth seeing in the graph, testing with `decree check`, or
  giving to a model or a person.
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
- **Shared code, config and data go in `.decree/lib/`** (`$DECREE_LIB`); `scripts/` holds only
  what states invoke. Project config goes in `.decree/env`; per-state values in the invoke's
  `env:`.
- **A step that waits for a shared resource is an invoked state with a `timeout`** (ComfyUI's
  queue, a model unloading), not an `onentry` script, which has none. Keep `onentry` for quick
  steps.
- **Never delete run folders to clean up**; they are the record. Use
  `decree prune --older-than <age>`.
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

## The built-in `develop`

`decree init` writes one working machine besides the router, `develop`: `precheck` →
`implement` → `gate` → `verify`, with `fix` and `final_gate` only when the gate fails. Its scripts
source `lib/ai.sh` (the `ai` function for the chosen `--ai`; add another backend there) and
follow the run-directory files in `reference/scripts.md` (`progress.md`, `STOP`, `gate.log`,
`plan.md`). The project fills in `scripts/develop/gate.sh` with its own checks; until then it
runs nothing and says so. `verify` ends in `pass` or `fail` from the AI's `VERDICT:` line.
Start from it before writing a new develop-style machine.

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

A machine grows one reason at a time. First, the straight line that does the job:

```yaml
# yaml-language-server: $schema=../schema/v1/machine.schema.json
# Graph: ../graph/develop.md
name: develop
description: Implement a message, then test it.
initial: implement
states:
  implement:                       # the job needs two scripts: one state each, in a line
    invoke: implement
    transitions: { done: test }
  test:
    invoke: test
    transitions: { done: done }
  done:   { final: true }
  failed: { final: true }
```

Real runs then fail at random in `implement`, and a re-run passes. The cheapest fix is
`attempts`:

```yaml
# yaml-language-server: $schema=../schema/v1/machine.schema.json
# Graph: ../graph/develop.md
name: develop
description: Implement a message, then test it.
initial: implement
states:
  implement:
    invoke:                        # runs fail at random and a re-run passes: try 3 times
      script: { name: implement, attempts: 3 }
    transitions: { done: test }
  test:
    invoke: test
    transitions: { done: done }
  done:   { final: true }
  failed: { final: true }
```

Later the local model often fails where Claude would not. Give each attempt a value instead
of adding a state:

```yaml
# yaml-language-server: $schema=../schema/v1/machine.schema.json
# Graph: ../graph/develop.md
name: develop
description: Implement a message, then test it.
initial: implement
states:
  implement:
    invoke:                        # the local model fails often: try Claude third
      script: { name: implement, attempts: [local, local, claude] }
    transitions: { done: test }
  test:
    invoke: test
    transitions: { done: done }
  done:   { final: true }
  failed: { final: true }
```

The script picks the model from the attempt: a choice of *how*, not of what runs next.

```sh
case "$DECREE_ATTEMPT_VALUE" in
  claude) claude -p "$(cat "$DECREE_MESSAGE")" ;;
  *)      ollama run qwen3:8b "$(cat "$DECREE_MESSAGE")" ;;
esac
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
