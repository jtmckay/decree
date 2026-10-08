# Decree

**Durable state-machine workflows from plain files.**

Work arrives as a message, a YAML statechart decides what happens next, and scripts do the work. Every step is written to an append-only log, so a run survives crashes, restarts and week-long waits for a reply, and continues where it stopped: durable execution in the spirit of [Temporal](https://temporal.io) or AWS Step Functions, but local, with one binary and a folder of files instead of a server and a database.

decree works with no AI at all. When you do use AI, decree is the harness around it: a model, or a person, only ever picks among the options a state declares, and the state machine stays in charge of what happens next.

decree is developed with itself: every change to this repository is a migration in [`.decree/migrations/`](.decree/migrations/), run by `decree process`.

decree is built from three blocks:

- **Messages** are markdown files that say *what* to do. The body is the task; the frontmatter names the machine that does it. One message is one run.
- **Machines** are YAML statecharts that say *in what order*: states, what each state invokes, and which event leads where. They follow [W3C SCXML](https://www.w3.org/TR/scxml/), written in YAML, and contain no code and no paths.
- **Scripts** are executables that do *one piece of work* and report one outcome. Bash by default.

Decisions are states of their own, cheapest first: a deterministic `check`, then a `model` (a local classifier or an LLM), then a `person`. Everything else is deterministic, and every step is recorded in each run's `events.jsonl`.

```text
            decree emit
  +-------------------------------+
  |                               v
  |   Message   (frontmatter = structured, body = unstructured)
  |      | names machine; decree mirrors state
  |      v
  |   Machine   (states, invokes, transitions)   <-- options / one event -->  model or person
  |      | script name + DECREE_* env       ^ event: exit code or $DECREE_EVENT_FILE
  |      v                                  |
  +-- Scripts   (scripts/<name>, bash by default)
```

## Install

```bash
cargo install decree
```

decree runs on Linux and macOS.

**Coming from 0.4?** 0.5 is a rewrite, and there is no upgrade path or upgrade tool: finish pending 0.4 work with 0.4 first. 0.5 does not read a 0.4 `.decree/`; what changed is in the [changelog's 0.5.0 entry](CHANGELOG.md#050---unreleased).

## Quick start

Every command in this section runs as written, in order, in an empty directory.

**1. Set up the project.**

```bash
decree init
```

This writes `.decree/` with the `develop` machine and its scripts, the `router` machine for your AI tool (`--ai claude|copilot|opencode`; by default the first one found on `PATH`), `lib/ai.sh` (the `ai` function the scripts source), and the decree skill for Claude Code or Copilot. `develop` implements a message with your AI in logged steps, runs your project's checks, has the AI fix them only if they fail, then has it check the acceptance criteria and name `pass` or `fail`. Fill in its gate, `.decree/scripts/develop/gate.sh` (commented lines show cargo, npm and Go, each with the `command -v` line that checks its tool); until you do, it runs nothing, says so and names `pass`. The gate names `pass` or `fail` from your checks; when they cannot run at all (a tool missing from `PATH`, say) it exits non-zero, and the run ends in `failed` without asking the AI to fix code that may be fine.

**2. Write a script.** A script does one thing. Exit 0 is the event `done`, anything else is `error`.

```bash
cat > .decree/scripts/greet.sh <<'EOF'
#!/usr/bin/env bash
echo "Hello from $DECREE_MACHINE: $(sed -n 's/^# //p' "$DECREE_MESSAGE" | head -n 1)"
EOF
cat > .decree/scripts/ask_person.sh <<'EOF'
#!/usr/bin/env bash
echo "$DECREE_QUESTION Reply with: decree event $DECREE_WAIT_ID <event>"
EOF
chmod +x .decree/scripts/greet.sh .decree/scripts/ask_person.sh
```

**3. Write a machine.** It names scripts, never paths: `greet` resolves to `scripts/hello/greet*`, else `scripts/greet*`. Editors that read SchemaStore apply the machine schema by path, so they complete keys and underline mistakes ([editors](docs/editors.md)).

```bash
cat > .decree/machines/hello.yml <<'EOF'
name: hello
description: Greet, ask a person to approve, then finish.
initial: greet
states:
  greet:
    invoke: greet
    transitions: { done: approval }
  approval:
    invoke:
      person:
        question: Approve the greeting?
        ask: ask_person
    transitions:
      approve: { target: done, description: Keep it. }
      reject:  { target: rejected, description: Throw it away. }
  done:     { final: true }
  rejected: { final: true }
  failed:   { final: true }
EOF
```

**4. Write a message.** Migrations are messages that run once, in filename order. A migration's run id is its file stem.

```bash
cat > .decree/migrations/01-hello.md <<'EOF'
---
machine: hello
---
# Say hello
EOF
```

**5. Check, graph and run it.**

```bash
decree check
decree graph
decree process
```

`decree check` validates every machine and pending message. `decree graph` writes a Mermaid diagram per machine to `.decree/graph/`. `decree process` runs `greet`, then pauses in `approval` and prints the `decree event` commands that answer it.

**6. Reply, and look at the record.**

```bash
decree event 01-hello approve
decree process
decree status
decree status 01-hello
```

The reply is a message too: `decree event` writes it to `inbox/`, and the next `process` delivers it. `decree status 01-hello` shows the run's events: transitions, scripts with durations, the wait and the reply. `01-hello.md` is now in `.decree/processed.md`, so it never runs again.

**7. Queue more work.** Any script (or you) can queue a message with `decree emit`; the body comes from stdin.

```bash
echo "# Greet again" | decree emit --machine hello
decree process
```

## Messages

A message is a markdown file with YAML frontmatter. decree reads and writes only the frontmatter and never changes the body.

```markdown
---
machine: develop            # machines/develop.yml; required
params:                     # sets the machine's data for this run
  max_rounds: 3
---
# Add rate limiting to /api/upload
Given ... When ... Then ...
```

Messages come from four places, and all of them run the same way:

| Source | Where | `trigger` |
| --- | --- | --- |
| Migrations | `.decree/migrations/*.md`, committed, never edited. Run once, in filename order; the ledger is `processed.md`. A failed one blocks the ones after it until `decree process --retry`. | `migration` |
| Inbox | `.decree/inbox/*.md`, run first-in, first-out by filename. Write a file directly, or use `decree emit`. | `inbox`, `emit` |
| Cron | `.decree/cron/*.md` templates with a `cron:` expression, queued by `decree daemon`. | `cron` |
| Sub-machines | A state with `invoke: { machine: <name> }` starts a child run. | `invoke` |

When decree claims a message it moves it to `.decree/runs/<id>/message.md`, adds `id` and `trigger` if they are missing, and mirrors the run's current `state` into it. `runs/<id>/events.jsonl` is the record: the run's state is always the last transition in it.

A reply to a waiting run is a message with `to:` (the wait id or run id) and `event:`. `decree event` writes one, and so can any tool.

## Machines

A machine is an SCXML statechart written in YAML, in `.decree/machines/<name>.yml`. Keys use SCXML's names: `initial`, `states`, `transitions`, `target`, `type`, `onentry`, `onexit`, `invoke`, `data`, `final`.

```yaml
# Graph: ../graph/deploy.md
name: deploy
description: Build, ask a person to approve, then ship.
initial: build
states:
  build:
    invoke: build               # scripts/deploy/build*, else scripts/build*
    transitions: { done: approval }
  approval:
    invoke:
      person:
        question: Ship it?
        ask: ask_person
    transitions:
      approve: { target: ship, description: Ship this build. }
      reject:  { target: rejected, description: Do not ship. }
  ship:
    invoke:
      script: { name: ship, attempts: 2 }   # re-run on a non-zero exit
    transitions: { done: done }
  done:     { final: true }
  rejected: { final: true }
  failed:   { final: true }     # every machine has one; unhandled errors go here
```

Every state does one thing: it invokes a function, and the function's result is the event that picks the next state. `invoke` has exactly one key, which names the kind:

| `invoke:` | What happens | Events |
| --- | --- | --- |
| `<script>`, or `script: { name: <script>, attempts: 2, timeout: 10m }` | Runs the script, re-running it up to `attempts` times and stopping it after `timeout` (a [duration](docs/reference/machines.md#durations): `90s`, `10m`, `12h`, `7d`). | `done`, `error`, or the event it writes to `$DECREE_EVENT_FILE` |
| `check: <condition>` | A deterministic condition over a state's output, `data`, `visits` or a model's confidence, such as `check: { visits: fix, less_than: 3 }`. | `true`, `false` |
| `model: { question: ..., output: <state> }` | A router machine asks a model to pick one of the state's transitions, with a confidence, given the `output` state's output and the message body. Below `min_confidence` the event is `unsure`. | the transition names, `unsure` |
| `person: { question: ..., ask: <script> }` | The `ask` script tells someone; the run pauses until a reply arrives. | the transition names, `error` on `timeout` |
| `machine: <name>`, or `machine: { name: <name>, params: {...} }` | Runs another machine as a child run. | the child's final state (`failed` as `error`) |

Other keys: `onentry` and `onexit` (scripts run on entering or leaving a state, or the whole run at the root), `data` (typed values set from a message's `params`), `emits` (machines a state's scripts may `decree emit` to), compound states (`initial` plus `states`, with transitions that bubble up) and `final`.

[The reference](docs/reference/README.md) is the full contract, [the decision log](docs/decisions.md) says why, and [`examples/`](examples/) holds worked projects with real files: start at [`examples/project/`](examples/project/README.md) for what a project's files look like. [`examples/newsletter/`](examples/newsletter/README.md) is the simplest one that does real work: three scripts in a straight line, run by cron.

### Graphs

`decree graph` writes `.decree/graph/<machine>.md` for every machine and `.decree/graph/system.md` for how machines emit to and invoke each other. Commit them.

1. Run `decree graph`, then open `.decree/graph/<machine>.md` (or `system.md`).
2. In VS Code, press `Ctrl+Shift+V` (`Cmd+Shift+V` on macOS) for the preview; VS Code 1.121 and later render Mermaid in Markdown without an extension. GitHub, GitLab and Obsidian render the committed files as they are.
3. Without any of those, copy the lines inside the `mermaid` fence into https://mermaid.live.

### Schema

decree has a JSON Schema for every file it reads or writes: machines, message frontmatter, `events.jsonl` lines, and a router's `request.json` and `reply.json`. They live in [`schema/v1/`](schema/v1/), published at `https://raw.githubusercontent.com/jtmckay/decree/main/schema/v1/`, and are versioned: within `v1` changes are additive only ([Versioning](docs/reference/README.md#versioning), [changelog](CHANGELOG.md)). Editors apply the machine schema to `.decree/machines/*.yml` through SchemaStore, or a setting until then, with no line in the file, so VS Code with Red Hat's YAML extension, JetBrains IDEs, or any editor running the YAML language server completes keys, shows what each one means and underlines mistakes as you type ([editors](docs/editors.md)). `decree schema` writes a local copy to `.decree/schema/v1/`, ignored by git, for agents and offline use: a model reads it as the contract to write machines against. The schema checks shape; `decree check` also checks that names resolve, targets exist and every state is reachable ([Schema](docs/reference/machines.md#schema)).

### Routers

A `model` state asks a **router**: an ordinary machine that reads `request.json`, asks a model however it likes and writes `reply.json`. decree validates the reply against the state's transitions, so the model never names a state. A state names its router with `router:`; without one it uses the machine named `router`, which `decree init` writes to ask Claude, Copilot or OpenCode (`scripts/router/ask_<ai>.sh`). Routers are typed (a classifier such as GLiNER2.5-Decide, or a model held to the request's `reply_schema`, which cannot answer outside the options) or untyped (a chat model such as `claude -p`, whose free-text reply is checked after the fact): use typed routers for routing and untyped models for the work, as [docs/routers.md](docs/routers.md) explains.

## Scripts

A script is any executable file. decree runs it directly from the project root, so the shebang picks the language.

Script `X` used by machine `M` is the first match of `X` or `X.<ext>` in:

1. `.decree/scripts/M/`
2. `.decree/scripts/`

An invoked script's event is `error` on a non-zero exit. On exit 0 it is `done`, unless the script writes an event name to the file `$DECREE_EVENT_FILE` (`echo pass > "$DECREE_EVENT_FILE"`). Output goes to `runs/<id>/<NNNN>-<state>-<script>.log` and is only a log: decree never reads an event from it. Scripts get the run's context in `DECREE_*` variables: `DECREE_MESSAGE`, `DECREE_MACHINE`, `DECREE_STATE`, `DECREE_ATTEMPT`, `DECREE_DATA_<NAME>` and more (`decree help` lists them all). A run's own files go in `$DECREE_RUN_DIR`; what a machine keeps between runs goes in `$DECREE_STORE` and is declared under the machine's `store:`, so the machine file says what it remembers; shared code and config go in `$DECREE_LIB`.

Model servers and other long-running processes are not scripts and decree does not manage them: an `onentry` script starts what a state needs. [docs/services.md](docs/services.md) shows systemd units, llama-swap and a tmux layout for that.

An agent a script starts runs with your own configuration, because decree adds nothing between them. The built-in machines call plain `claude -p` (with `--permission-mode`, default `auto`, set by `CLAUDE_PERMISSION_MODE`), so your Claude Code settings, hook scripts and skills apply to every agent step. A token filter such as [rtk](https://github.com/rtk-ai/rtk), which shortens the output of the agent's own shell commands through a Claude Code hook, therefore needs no setup in decree: install its hook once and every run uses it. It does not filter files the agent reads, such as the `gate.log` that `develop`'s `fix` step reads, and decree's own logs stay complete.

## Commands

| Command | What it does |
| --- | --- |
| `decree init [--ai AI] [--permissions]` | Create `.decree/` with machines, scripts, the `router` machine and the decree skill |
| `decree check [--format text\|json\|sarif]` | Validate machines and pending messages; `sarif` for code scanning |
| `decree graph` | Write Mermaid diagrams to `.decree/graph/` |
| `decree schema` | Write a local copy of the JSON Schemas for machines, messages, events, router files and `--format json` output to `.decree/schema/v1/` |
| `decree skill [--ai AI]` | Refresh the decree skill in `.claude/skills/decree/` or `.github/skills/decree/` after an upgrade |
| `decree process [--dry-run]` | Deliver replies, continue pending runs, drain `inbox/`, then run pending migrations in order |
| `decree process --retry [ID] [--state S]` | First continue a failed or interrupted run: `ID`, or the migration blocking the queue |
| `decree daemon [--interval D]` | The same passes plus cron, every `D` (default `2s`) |
| `decree emit --machine M [--param K=V]...` | Queue a message for `M`, body from stdin; prints its id |
| `decree event ID EVENT [-m NOTE]` | Reply to a run waiting for a person |
| `decree status [ID]` | Runs by status and queued messages; one run's events |
| `decree status --cron` | Cron files and when each fires next |
| `decree tail [ID]` | Follow the live output of a run |
| `decree prune --older-than AGE [--dry-run]` | Delete finished runs older than `AGE` (`30d`, `12h`, `90m`, `90s`); ship them to Loki first if you keep history |
| `decree help` | Full reference: files, keys, environment variables |
| `decree --version` | Print the version |

`--format json` on `check`, `status`, `emit`, `event`, `prune`, `graph`, `schema` and `process --dry-run` prints one JSON document, described by `schema/v1/cli/<command>.schema.json`, for CI and agents; exit codes do not change. `decree check --format sarif` prints a SARIF 2.1.0 log for GitHub code scanning, GitLab or Azure DevOps ([Machine-readable output](docs/reference/cli.md#machine-readable-output) has a GitHub Actions job).

decree never continues a run that was stopped by a signal or a crash: the run is `interrupted` until you run `decree process --retry`, because a kill may be deliberate. A failed migration blocks the ones after it the same way, and `decree process` prints the exact command that continues it.

## Files

```text
.decree/
  .gitignore                      # inbox/, runs/ and store/
  env                             # KEY=value for every script; the process environment wins
  machines/<name>.yml             # machines
  scripts/<name>                  # scripts shared by every machine
  scripts/<machine>/<name>        # a machine's own scripts, found first
  lib/                            # code, config and data scripts source ($DECREE_LIB); never run by decree
  migrations/                     # ordered, run-once messages; committed, never edited
  processed.md                    # ledger of migrations that ran; committed
  inbox/                          # queued messages
  cron/                           # message templates queued on a schedule
  runs/<id>/                      # one folder per run: message.md, events.jsonl, traces.jsonl, logs
  store/<machine>/                # what a machine keeps between runs ($DECREE_STORE), declared under its store:; never pruned
  graph/                          # written by decree graph; committed
  schema/v1/                      # optional local copy written by decree schema; not committed
```

There is no configuration file. Each setting is a convention or a fixed limit:

| Setting | Instead |
| --- | --- |
| Router for a `model` invoke | The machine named `router`, unless the invoke sets `router:` |
| Default machine | None: every message names its `machine:` |
| Retries | `attempts` in the script invoke, a number or a list of values (`[local, claude]`); default 1 (no retry) |
| Emit depth | A fixed limit of 10 |
| Log size | Each script log is capped at 2 MiB |
| Sharing across projects | Symlink shared machines and scripts into `machines/` and `scripts/` |

A cron file (`.decree/cron/<name>.md`, queued by `decree daemon`):

```markdown
---
cron: "0 9 * * 1-5"
machine: develop
---
Run the weekday morning task.
```

## Observability

`events.jsonl` is one JSON line per event, with `run_id`, `machine` and `type` on each, so any log shipper can read it. Each run is also an OpenTelemetry trace: spans for the run, its scripts, decisions and child runs go to `traces.jsonl` as OTLP/JSON, scripts get `TRACEPARENT` so their own instrumented calls join the trace, and a message's `traceparent` continues an upstream one ([Traces](docs/reference/observability.md#traces)). [`examples/observability/`](examples/observability/README.md) ships the events to Loki with Grafana Alloy, with example queries for Grafana, and the traces to Jaeger with the OpenTelemetry Collector.

## Security

decree runs scripts and Claude with your permissions and no sandbox; read the [security policy](SECURITY.md) before processing messages you did not write.

## License

[MIT](LICENSE)
