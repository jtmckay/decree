# Decree

**Durable state-machine workflows from plain files.**

Work arrives as a message, a YAML statechart decides what happens next, and scripts do the work. Every step is written to an append-only log, so a run survives crashes, restarts and week-long waits for a reply, and continues where it stopped: durable execution in the spirit of [Temporal](https://temporal.io) or AWS Step Functions, but local, with one binary and a folder of files instead of a server and a database.

decree works with no AI at all. When you do use AI, decree is the harness around it: a model, or a person, only ever picks among the options a state declares, and the state machine stays in charge of what happens next.

decree is developed with itself: every change to this repository is an order file in `.decree/machines/develop/order/`, run by `decree daemon --once`.

decree is built from three blocks:

- **Messages** are markdown files that say *what* to do. The body is the task; the frontmatter names the machine that does it. One message is one run.
- **Machines** are YAML statecharts that say *in what order*: states, what each state invokes, and which event leads where. They follow [W3C SCXML](https://www.w3.org/TR/scxml/), written in YAML, and contain no code and no paths.
- **Scripts** are executables that do *one piece of work* and report one outcome. Bash by default.

Decisions are states of their own, cheapest first: a deterministic `check`, then a `model` (a local classifier or an LLM), then a `reply` from a person. Everything else is deterministic, and every step is recorded in each run's `events.jsonl`.

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
curl -fsSL https://raw.githubusercontent.com/jtmckay/decree/main/install.sh | sh
```

This installs the newest stable release from 0.5.0 on (while there is none, the newest beta) as a static Linux x86-64 binary in `$XDG_BIN_HOME` (default `~/.local/bin`), with no sudo, after checking its sha256; run it again to update, or pin a version with `| sh -s -- v0.5.0`. Elsewhere (macOS, ARM), build from a tag: `cargo install --git https://github.com/jtmckay/decree --tag v0.5.0`. crates.io has only an older decree until 0.5.0 is published there, so `cargo install decree` is not this version.

decree runs on Linux and macOS.

**Coming from 0.4?** 0.5 is a rewrite, and there is no upgrade path or upgrade tool: finish pending 0.4 work with 0.4 first. 0.5 does not read a 0.4 `.decree/`; what changed is in the [changelog's 0.5.0 entry](CHANGELOG.md#050---2026-10-19).

## Quick start

Every command in this section runs as written, in order, in an empty directory.

**1. Set up the project.**

```bash
decree init
```

This writes `.decree/`: the empty queues, a `.gitignore`, a `.env.example` (`AI_CLI`, which `--ai claude|copilot|opencode` sets), the official plugins `ai`, `order`, `prune` and `skill`, and the decree skill for Claude Code or Copilot. It asks nothing: it ends by listing the other official plugins and the line that adds one.

**2. Add an official plugin.** A plugin is one machine and its scripts, kept in one file, `.decree/plugins/<name>.md` ([Plugins](docs/reference/plugins.md)). decree is built with its official plugins, so adding one needs no network:

```bash
decree list
decree install develop
```

`decree install develop` copies `develop` and `infra`, which it needs, into `.decree/plugins/` and installs each into its folder, `.decree/machines/<name>/`. Commit `.decree/plugins/`, `.decree/system.md` and each machine's `graph.md`: `.decree/.gitignore` ignores the rest of what is installed. decree installs a plugin only when asked: in a fresh clone, or after a pull that changes a plugin, run `decree install` (`decree check` and `decree daemon --once` say when one is due), or add the official plugin `plugin_install` to do it each time the daemon starts. To change a plugin, edit its installed copy, such as the checks in `.decree/machines/develop/scripts/gate.sh`, then keep the edit with `decree export develop`; a run's `commit` exports for you, so what a run changes is in its one commit. `decree list` marks a plugin `differs` when your copy is not the one this decree ships, and `decree install --force develop` takes this decree's copy, printing the edits it replaces. An install never stops over a change: an installed copy edited in place, and a plugin file it replaces, go to `.decree/archive/` first.

`develop` implements a message with an AI agent, runs your checks and has the agent check the acceptance criteria. For a Rust, Node or React project, add `develop_rust`, `develop_node` or `develop_react` instead: `develop` with a gate that already runs that ecosystem's checks (cargo fmt, clippy and test; the lockfile's package manager running `lint`, `typecheck`, `test`, and for React `build`) and prompts that follow its conventions. Its scripts ask through the `ai` plugin's `ai.sh`, which `decree install` brings along (the `ai` function, with a function per backend: it uses the run's `ai_cli` data, else `AI_CLI` in `.decree/.env`, else the first of `claude`, `opencode` and `copilot` on `PATH`; it waits out Claude's usage limit and logs every call in `.decree/ai-use.md`, as [The AI helper](docs/reference/scripts.md#the-ai-helper) says). It works in a git repository: a run starts only from a clean work tree, and a run that ends in `done` is one commit, titled with the message's first heading ([D63](docs/decisions.md#d63-one-commit-per-run-from-a-clean-tree)). Commit or stash your own changes before queuing work. A run that ends in `failed` keeps its changes in the work tree, and its file in `.decree/attention/` holds the queue, so no other run starts on them: `decree retry <id>` continues the run on its own work and removes the hold, or stash the changes and delete the file to set them aside. That folder has a file for every run that needs a person (failed, interrupted, or waiting for a reply), with the command that continues it; `decree daemon --once` and `decree status` point to it ([Attention](docs/reference/cli.md#attention), [D67](docs/decisions.md#d67-the-attention-folder-says-what-needs-a-person)). For divergent work, or to keep editing while runs go on, run decree in a git worktree you make: `git worktree add ../app-exp -b exp`, then `decree daemon --once` there. Each checkout has its own runs and inbox, and its branch its own `order.git.txt` records, so the two never meet until you merge. A worktree holds committed files only: copy `.decree/.env` (and any app `.env`) in, and run `.decree/machines/infra/scripts/run.sh apply` there first.

`infra` changes the environment the project needs, the first time and every time after: the AI declares each resource in `.decree/machines/infra/scripts/requirements` (`command python3`, `pip hypothesis 6.112.0`), and a script installs what is missing and checks every line, as Ansible applies a play. Nothing runs `sudo`; a step only you can do prints the command and fails until you run it. `develop` checks the same file, and any `requirements` beside another machine's scripts, before it starts ([Declared environment](docs/reference/scripts.md#declared-environment)).

`qa` (`decree install qa`) tries to break the project and hands what breaks to `develop`. The AI lists the attack surface, writes chaos tests in `tests/chaos/` against the invariants in `.decree/machines/qa/scripts/invariants.md`, using the wrenches in `scripts/qa/wrenches.md` (bad input, repetition, interruption, full disks, corrupt state, …), and runs them. Triage marks each real bug's tests expected to fail and writes one report per root cause, stating the broken invariant and the expected behaviour and proposing no fix, and each report becomes a `develop` message. QA never fixes code: a script fails the step if the AI changed anything outside the chaos tests. Set `QA_RUN` in `.decree/machines/qa/scripts/qa.sh` to the command that runs the chaos suite first (and keep it with `decree export qa`), and add it to the gate once qa finds bugs, so a fixed bug whose test is still marked fails the gate ([D62](docs/decisions.md#d62-qa-finds-bugs-and-never-fixes-them)).

The other official plugins are `router` (the default router a `model` state asks), `newsletter`, `comfy` (renders on ComfyUI), `develop_by_size` with its routers `gliner_router` and `llm_router`, `lease` (one holder at a time for a GPU or another shared resource, batched by mode), `registry` (where each service is, and which lease to get before using it), and `plugin_install` (runs `decree install` each time the daemon starts); each plugin's file says what it needs and how to use it.

**3. Write a script.** A script does one thing. Exit 0 is the event `done`, anything else is `error`. A machine's scripts live in its own folder, `.decree/machines/<name>/scripts/`.

```bash
mkdir -p .decree/machines/hello/scripts
cat > .decree/machines/hello/scripts/greet.sh <<'EOF'
#!/usr/bin/env bash
echo "Hello from $DECREE_MACHINE: $(sed -n 's/^# //p' "$DECREE_MESSAGE" | head -n 1)"
EOF
cat > .decree/machines/hello/scripts/notify_person.sh <<'EOF'
#!/usr/bin/env bash
echo "$DECREE_QUESTION Reply with: decree reply $DECREE_WAIT_ID <event>"
EOF
chmod +x .decree/machines/hello/scripts/*.sh
```

**4. Write a machine.** It names scripts, never paths: `greet` resolves to `.decree/machines/hello/scripts/greet*`. Editors that read SchemaStore apply the machine schema by path, so they complete keys and underline mistakes ([editors](docs/editors.md)).

```bash
cat > .decree/machines/hello/machine.yml <<'EOF'
name: hello
description: Greet, ask a person to approve, then finish.
initial: greet
states:
  greet:
    invoke: greet
    transitions: { done: approval }
  approval:
    invoke:
      reply:
        question: Approve the greeting?
        notify: notify_person
    transitions:
      approve: { target: done, description: Keep it. }
      reject:  { target: rejected, description: Throw it away. }
  done:     { final: true }
  rejected: { final: true }
  failed:   { final: true }
EOF
```

**5. Queue a message.** `decree emit` writes a message for `hello` to `.decree/inbox/` and prints its run id; keep it to answer the run.

```bash
id=$(echo "# Say hello" | decree emit --machine hello)
```

**6. Keep, check, graph and run it.**

```bash
decree export hello
decree check
decree graph
decree daemon --once
```

`decree export hello` keeps the machine you wrote as a plugin, `.decree/plugins/hello.md`, with the two scripts it runs, so it is committed. `decree check` validates every machine and queued message. `decree graph` writes a Mermaid diagram per machine to its folder, `.decree/machines/<name>/graph.md`. `decree daemon --once` runs `greet`, then pauses in `approval` and prints the `decree reply` commands that answer it.

**7. Reply, and look at the record.**

```bash
decree reply "$id" approve
decree daemon --once
decree status
decree status "$id"
```

The reply is a message too: `decree reply` writes it to `inbox/`, and the next `decree daemon --once` delivers it. `decree status "$id"` shows the run's events: transitions, scripts with durations, the wait and the reply.

**8. Queue more work.** Any script (or you) can queue a message with `decree emit`; the body comes from stdin, or a whole message (`decree emit < .decree/runs/<id>/message.md` runs one again).

```bash
echo "# Greet again" | decree emit --machine hello
decree daemon --once
```

## Messages

A message is a markdown file with YAML frontmatter. decree reads and writes only the frontmatter and never changes the body.

```markdown
---
machine: develop            # machines/develop/machine.yml; required
params:                     # sets the machine's data for this run
  max_rounds: 3
---
# Add rate limiting to /api/upload

## Overview
## Outcome
## Constraints
## Acceptance Criteria
```

Messages come from four places, and all of them run the same way:

| Source | Where | `trigger` |
| --- | --- | --- |
| Order files | `.decree/machines/<machine>/order/*.md`, committed. The official `order` plugin runs them at each daemon start, one at a time in filename order, each once and again when it changes; a failed one stops the ones after it. | `invoke` |
| Inbox | `.decree/inbox/*.md`, run first-in, first-out by filename. Write a file directly, or use `decree emit`. | `inbox`, `emit` |
| Cron | `.decree/machines/<machine>/cron/*.md` templates with a `cron:` expression, queued by `decree daemon`. | `cron` |
| Sub-machines | A state with `invoke: { machine: <name> }` starts a child run. | `invoke` |

When decree claims a message it moves it to `.decree/runs/<id>/message.md`, adds `id` and `trigger` if they are missing, and mirrors the run's current `state` into it. `runs/<id>/events.jsonl` is the record: the run's state is always the last transition in it.

A reply to a waiting run is a message with `to:` (the wait id or run id) and `event:`. `decree reply` writes one, and so can any tool.

## Machines

A machine is an SCXML statechart written in YAML, in `.decree/machines/<name>/machine.yml`. Keys use SCXML's names: `initial`, `states`, `transitions`, `target`, `type`, `onentry`, `onexit`, `invoke`, `data`, `final`.

```yaml
# Graph: graph.md
name: deploy
description: Build, ask a person to approve, then ship.
initial: build
states:
  build:
    invoke: build               # scripts/deploy/build*, else scripts/build*
    transitions: { done: approval }
  approval:
    invoke:
      reply:
        question: Ship it?
        notify: notify_person
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
| `reply: { question: ..., notify: <script> }` | The `notify` script tells someone (a person, a UI, a program); the run pauses until a reply arrives. | the transition names, `error` on `timeout` |
| `machine: <name>`, or `machine: { name: <name>, params: {...} }` | Runs another machine as a child run. | the child's final state (`failed` as `error`) |

Other keys: `onentry` and `onexit` (scripts run on entering or leaving a state, or the whole run at the root), `data` (typed values set from a message's `params`), `emits` (machines a state's scripts may `decree emit` to), compound states (`initial` plus `states`, with transitions that bubble up) and `final`.

[The reference](docs/reference/README.md) is the full contract, [the decision log](docs/decisions.md) says why, and [`examples/`](examples/) holds worked projects with real files: start at [`examples/project/`](examples/project/README.md) for what a project's files look like. The official plugins in [`plugins/`](plugins/) are machines that do real work; [`newsletter`](plugins/newsletter.md) is the simplest: three scripts in a straight line, run by cron.

### Graphs

`decree graph` writes `.decree/machines/<machine>/graph.md` for every machine and `.decree/system.md` for how machines emit to and invoke each other. Commit them.

1. Run `decree graph`, then open `.decree/machines/<machine>/graph.md` (or `.decree/system.md`).
2. In VS Code, press `Ctrl+Shift+V` (`Cmd+Shift+V` on macOS) for the preview; VS Code 1.121 and later render Mermaid in Markdown without an extension. GitHub, GitLab and Obsidian render the committed files as they are.
3. Without any of those, copy the lines inside the `mermaid` fence into https://mermaid.live.

### Schema

decree has a JSON Schema for every file it reads or writes: machines, message frontmatter, `events.jsonl` lines, and a router's `request.json` and `reply.json`. They live in [`schema/v1/`](schema/v1/), published at `https://raw.githubusercontent.com/jtmckay/decree/main/schema/v1/`, and are versioned: within `v1` changes are additive only ([Versioning](docs/reference/README.md#versioning), [changelog](CHANGELOG.md)). Editors apply the machine schema to `.decree/machines/*.yml` through SchemaStore, or a setting until then, with no line in the file, so VS Code with Red Hat's YAML extension, JetBrains IDEs, or any editor running the YAML language server completes keys, shows what each one means and underlines mistakes as you type ([editors](docs/editors.md)). `decree init` and `decree install` write a local copy to `.decree/schema/v1/`, ignored by git, for agents and offline use: a model reads it as the contract to write machines against. The schema checks shape; `decree check` also checks that names resolve, targets exist and every state is reachable ([Schema](docs/reference/machines.md#schema)).

### Routers

A `model` state asks a **router**: an ordinary machine that reads `request.json`, asks a model however it likes and writes `reply.json`. decree validates the reply against the state's transitions, so the model never names a state. A state names its router with `router:`; without one it uses the machine named `router`, which the official `router` plugin (`decree install router`) provides: it asks Claude, OpenCode or Copilot, whichever `AI_CLI` names or is first on `PATH`. Routers are typed (a classifier such as GLiNER2.5-Decide, or a model held to the request's `reply_schema`, which cannot answer outside the options) or untyped (a chat model such as `claude -p`, whose free-text reply is checked after the fact): use typed routers for routing and untyped models for the work, as [docs/routers.md](docs/routers.md) explains.

## Scripts

A script is any executable file. decree runs it directly from the project root, so the shebang picks the language.

Script `X` used by machine `M` is the one match of `X` or `X.<ext>` in `.decree/machines/M/scripts/`; there is no other search path. Code several machines share is a library: a machine folder with no `machine.yml`, whose scripts others source by path.

An invoked script's event is `error` on a non-zero exit. On exit 0 it is `done`, unless the script writes an event name to the file `$DECREE_EVENT_FILE` (`echo pass > "$DECREE_EVENT_FILE"`). Output goes to `runs/<id>/<NNNN>-<state>-<script>.log` and is only a log: decree never reads an event from it. Scripts get the run's context in `DECREE_*` variables: `DECREE_MESSAGE`, `DECREE_MACHINE`, `DECREE_STATE`, `DECREE_ATTEMPT`, `DECREE_DATA_<NAME>` and more (`decree help` lists them all). A run's own files go in `$DECREE_RUN_DIR`; what a machine keeps between runs goes in `$DECREE_STATE_DIR` and is declared under the machine's `state_files:`, so the machine file says what it remembers; its helpers, config and data sit beside its scripts in `.decree/machines/<machine>/scripts/`.

Model servers and other long-running processes are not scripts and decree does not manage them: an `onentry` script starts what a state needs. [docs/services.md](docs/services.md) shows systemd units, llama-swap and a tmux layout for that.

An agent a script starts runs with your own configuration, because decree adds nothing between them. The official plugins call plain `claude -p` (with `--permission-mode`, default `auto`, set by `CLAUDE_PERMISSION_MODE`, and `--model` when `CLAUDE_MODEL` is set), so your Claude Code settings, hook scripts and skills apply to every agent step. A token filter such as [rtk](https://github.com/rtk-ai/rtk), which shortens the output of the agent's own shell commands through a Claude Code hook, therefore needs no setup in decree: install its hook once and every run uses it. It does not filter files the agent reads, such as the `gate.log` that `develop`'s `fix` step reads, and decree's own logs stay complete.

## Commands

| Command | What it does |
| --- | --- |
| `decree init [--ai AI] [--permissions]` | Create `.decree/`, add the default plugins (`ai`, `order`, `prune`, `skill`) and write the decree skill; list the other official plugins |
| `decree install [NAME...\|FILE\|-]` | Install every plugin in `.decree/plugins/` (after a clone or a pull), the named ones (seeding an official one), or a plugin file or `-` for stdin (`curl -fsSL URL \| decree install -`) |
| `decree list` | The plugins in `.decree/plugins/` (official or own, `differs`, install state), the machines no plugin installed, and the official plugins not yet installed |
| `decree export [NAME]` | Write `.decree/plugins/<name>.md` from an installed copy; with no name, every plugin edited in place |
| `decree check [--format text\|json\|sarif]` | Validate machines and pending messages; `sarif` for code scanning |
| `decree graph` | Write a Mermaid diagram per machine, `.decree/machines/<name>/graph.md`, and `.decree/system.md` |
| `decree daemon --once [--machine M \| --exclude-machine M]` | Start, fire `@reboot` cron files (so `order` runs the order files), deliver replies, continue pending runs and drain `inbox/`, then exit; `--machine` runs only those machines, `--exclude-machine` all but those, so several processes can share one folder |
| `decree daemon [--interval D] [--machine M \| --exclude-machine M]` | The same, plus scheduled cron, every `D` (default `2s`), until stopped: what unattended work runs under, since `--once` exits while a run waits for a person |
| `decree emit [--machine M] [--param K=V]...` | Queue a message from stdin, a body for `M` or a whole message; prints its id |
| `decree reply ID EVENT [-m NOTE]` | Reply to a run waiting for a person |
| `decree retry ID [--state S]` | Make a failed or interrupted run pending again; the daemon, or the next `decree daemon --once`, continues it |
| `decree cancel ID [-m NOTE]` | End a waiting, pending or interrupted run, and the child runs below it, in `failed`; no script runs |
| `decree pause [-m NOTE]` | Let the runs going now finish their step, then run nothing more until `decree resume`; Ctrl-C interrupts the runs instead |
| `decree resume` | Go on after `decree pause`; a running daemon needs no restart |
| `decree status [ID]` | What needs attention, runs by status and queued messages; one run's events |
| `decree status --cron` | Cron files and when each fires next |
| `decree tail [ID]` | Follow the live output of a run |
| `decree upgrade` | Print how to upgrade decree: the install line, pinned to a release, and for a `cargo install` copy; then `decree install` in each project, and `decree list` to see which plugins the new version changed. Then it asks before running the install line (`--yes`: without asking) |
| `decree help` | Full reference: files, keys, environment variables |
| `decree` | Print a synopsis of the quick start; it runs nothing (`decree daemon --once` runs the queue) |
| `decree --version` | Print the version |

`--format json` on `check`, `status`, `emit`, `reply`, `retry`, `cancel`, `pause`, `resume`, `graph`, `upgrade`, `install`, `list` and `export` prints one JSON document, described by `schema/v1/cli/<command>.schema.json`, for CI and agents; exit codes do not change. `decree check --format sarif` prints a SARIF 2.1.0 log for GitHub code scanning, GitLab or Azure DevOps ([Machine-readable output](docs/reference/cli.md#machine-readable-output) has a GitHub Actions job).

decree never continues a run that was stopped by a signal or a crash: the run is `interrupted` until you run `decree retry <id>`, because a kill may be deliberate; a running daemon then continues it, or the next `decree daemon --once` does. `decree daemon --once` prints the exact command that continues it.

## Files

```text
.decree/
  .gitignore                      # inbox/, runs/, attention/, what is installed in machines/, .env and other local files
  .env                            # KEY=value for every script; the process environment wins
  plugins/<name>.md               # one machine's folder as one file: machine, scripts, cron files, state and order seeds; committed
  machines/<name>/                # installed from plugins/ by `decree install`
    machine.yml                   # the machine
    scripts/                      # its scripts, and its helpers, config and data; decree runs only what a state names
    cron/                         # message templates queued on a schedule
    state/                        # what it keeps between runs ($DECREE_STATE_DIR); *.git.* committed, the rest not
    order/                        # its order files, run in order at each daemon start; committed
    graph.md                      # written by decree graph; committed
  system.md                       # written by decree graph; committed
  inbox/                          # queued messages
  runs/<id>/                      # one folder per run: message.md, events.jsonl, traces.jsonl, logs
  attention/<id>.md               # what needs a person, with the command that continues it
  schema/v1/                      # written by init and decree install; not committed
```

There is no configuration file. Each setting is a convention, a fixed limit or a variable:

| Setting | Instead |
| --- | --- |
| Router for a `model` invoke | The machine named `router`, unless the invoke sets `router:` |
| Default machine | None: every message names its `machine:` |
| Retries | `attempts` in the script invoke, a number or a list of values (`[local, claude]`); default 1 (no retry) |
| Emit depth | 10, unless `DECREE_MAX_DEPTH` in the shell or `.decree/.env` sets it |
| Log size | Each script log is capped at 2 MiB |
| Sharing across projects | A plugin, `.decree/plugins/<name>.md`: one file holding a machine and its scripts (`decree export`, `decree install`) |

A cron file (`.decree/machines/<machine>/cron/<name>.md`, queued by `decree daemon`; `machine:` defaults to the folder's):

```markdown
---
cron: "0 9 * * 1-5"
machine: develop
---
# The weekday morning task

## Overview
## Outcome
## Constraints
## Acceptance Criteria
```

## Observability

`events.jsonl` is one JSON line per event, with `run_id`, `machine` and `type` on each, so any log shipper can read it. Each run is also an OpenTelemetry trace: spans for the run, its scripts, decisions and child runs go to `traces.jsonl` as OTLP/JSON, scripts get `TRACEPARENT` so their own instrumented calls join the trace, and a message's `traceparent` continues an upstream one ([Traces](docs/reference/observability.md#traces)). [`examples/observability/`](examples/observability/README.md) ships the events to Loki with Grafana Alloy, with example queries for Grafana, and the traces to Jaeger with the OpenTelemetry Collector.

## Security

decree runs scripts and Claude with your permissions and no sandbox; read the [security policy](SECURITY.md) before processing messages you did not write.

## License

[MIT](LICENSE)
