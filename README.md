# Decree

Run work through state machines you can read, check and graph. decree is built from three blocks:

- **Messages** are markdown files that say *what* to do. The body is the task; the frontmatter names the machine that does it. One message is one run.
- **Machines** are YAML statecharts that say *in what order*: states, what each state invokes, and which event leads where. They follow [W3C SCXML](https://www.w3.org/TR/scxml/), written in YAML, and contain no code and no paths.
- **Scripts** are executables that do *one piece of work* and report one outcome. Bash by default.

A model or a person only ever picks among the transitions a machine declares, in a state that says `choose`. Everything else is deterministic, and every step is recorded in an append-only `events.jsonl` per run.

```text
            decree emit
  +-------------------------------+
  |                               v
  |   Message   (frontmatter = structured, body = unstructured)
  |      | names machine; decree mirrors state
  |      v
  |   Machine   (states, invokes, transitions)   <-- options / one event -->  model or person (choose)
  |      | script name + DECREE_* env       ^ event: exit code or JSON line
  |      v                                  |
  +-- Scripts   (scripts/<name>, bash by default)
```

## Install

```bash
cargo install decree
```

decree runs on Linux and macOS.

## Quick start

Every command in this section runs as written, in order, in an empty directory.

**1. Set up the project.**

```bash
decree init
```

This writes `.decree/` with the `develop` and `rust_develop` machines and their scripts, the `router` machine for your AI tool (`--ai claude|copilot|opencode`; by default the first one found on `PATH`), and the decree skill for Claude Code or Copilot.

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

**3. Write a machine.** It names scripts, never paths: `greet` resolves to `scripts/hello/greet*`, else `scripts/greet*`.

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
    invoke: { choose: person, question: "Approve the greeting?", ask: ask_person }
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
| Migrations | `.decree/migrations/*.md`, committed, never edited. Run once, in filename order; the ledger is `processed.md`. A failed one blocks the ones after it until `decree retry`. | `migration` |
| Inbox | `.decree/inbox/*.md`, run first-in, first-out by filename. Write a file directly, or use `decree emit`. | `inbox`, `emit` |
| Cron | `.decree/cron/*.md` templates with a `cron:` expression, queued by `decree daemon`. | `cron` |
| Sub-machines | A state with `invoke: { machine: <name> }` starts a child run. | `invoke` |

When decree claims a message it moves it to `.decree/runs/<id>/message.md`, adds `id`, `trigger` and `parent`, and mirrors the run's current `state` into it. `runs/<id>/events.jsonl` is the record: the run's state is always the last transition in it.

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
    invoke: { choose: person, question: "Ship it?", ask: ask_person }
    transitions:
      approve: { target: ship, description: Ship this build. }
      reject:  { target: rejected, description: Do not ship. }
  ship:
    invoke: ship
    max_attempts: 2             # re-run on a non-zero exit
    transitions: { done: done }
  done:     { final: true }
  rejected: { final: true }
  failed:   { final: true }     # every machine has one; unhandled errors go here
```

Every state does one thing: it invokes a function, and the function's result is the event that picks the next state.

| `invoke:` | What happens | Events |
| --- | --- | --- |
| `<script>` | Runs the script. | `done`, `error`, or the `event` of a JSON object on its last stdout line |
| `{ check: ... }` | A deterministic condition over a state's output, `data`, `visits` or a model's confidence, such as `{ check: { visits: fix, less_than: 3 } }`. | `yes`, `no` |
| `{ choose: model, question: "..." }` | A router machine asks a model to pick one of the state's transitions, with a confidence. Below `min_confidence` the event is `unsure`. | the transition names, `unsure` |
| `{ choose: person, question: "...", ask: <script> }` | The `ask` script tells someone; the run pauses until a reply arrives. | the transition names, `error` on `timeout_s` |
| `{ machine: <name> }` | Runs another machine as a child run. | the child's final state (`failed` as `error`) |

Other keys: `onentry` and `onexit` (scripts run on entering or leaving a state, or the whole run at the root), `data` (typed values set from a message's `params`), `emits` (machines a state's scripts may `decree emit` to), `timeout_s`, `max_attempts`, compound states (`initial` plus `states`, with transitions that bubble up) and `final`.

[The reference](docs/reference/README.md) is the full contract, [the decision log](docs/decisions.md) says why, and `mock/` is a worked example of every feature with real files (start at [`mock/README.md`](mock/README.md)).

### Graphs

`decree graph` writes `.decree/graph/<machine>.md` for every machine and `.decree/graph/system.md` for how machines emit to and invoke each other. Commit them.

1. Run `decree graph`, then open `.decree/graph/<machine>.md` (or `system.md`).
2. In VS Code, press `Ctrl+Shift+V` (`Cmd+Shift+V` on macOS) for the preview; VS Code 1.121 and later render Mermaid in Markdown without an extension. GitHub, GitLab and Obsidian render the committed files as they are.
3. Without any of those, copy the lines inside the `mermaid` fence into https://mermaid.live.

### Routers

A `choose: model` state asks a **router**: an ordinary machine that reads `request.json`, asks a model however it likes and writes `reply.json`. decree validates the reply against the state's transitions, so the model never names a state. A state names its router with `router:`; without one it uses the machine named `router`, which `decree init` writes to ask Claude, Copilot or OpenCode (`scripts/router/ask_<ai>.sh`). [docs/routers.md](docs/routers.md) shows routers for other models and local classifiers.

## Scripts

A script is any executable file. decree runs it directly from the project root, so the shebang picks the language.

Script `X` used by machine `M` is the first match of `X` or `X.<ext>` in:

1. `.decree/scripts/M/`
2. `.decree/scripts/`

An invoked script's event is `error` on a non-zero exit. On exit 0 it is `done`, unless the last stdout line is a JSON object such as `{"event":"pass"}`. Output goes to `runs/<id>/<NNNN>-<state>-<script>.log`. Scripts get the run's context in `DECREE_*` variables: `DECREE_MESSAGE`, `DECREE_MACHINE`, `DECREE_STATE`, `DECREE_ATTEMPT`, `DECREE_DATA_<NAME>` and more (`decree help` lists them all).

Model servers and other long-running processes are not scripts and decree does not manage them: an `onentry` script starts what a state needs. [docs/services.md](docs/services.md) shows systemd units, llama-swap and a tmux layout for that.

## Commands

| Command | What it does |
| --- | --- |
| `decree init [--ai AI] [--permissions]` | Create `.decree/` with machines, scripts, the `router` machine and the decree skill |
| `decree check` | Validate machines and pending messages |
| `decree graph` | Write Mermaid diagrams to `.decree/graph/` |
| `decree process [--dry-run]` | Deliver replies, continue pending runs, drain `inbox/`, then run pending migrations in order |
| `decree daemon [--interval S]` | The same passes plus cron, every `S` seconds |
| `decree emit --machine M [--param K=V]...` | Queue a message for `M`, body from stdin; prints its id |
| `decree event ID EVENT [-m NOTE]` | Reply to a run waiting for a person |
| `decree status [ID]` | Runs by status and queued messages; one run's events |
| `decree status --cron` | Cron files and when each fires next |
| `decree tail [ID]` | Follow the live output of a run |
| `decree retry ID [--state S]` | Make an interrupted or finished run pending again |
| `decree help` | Full reference: files, keys, environment variables |
| `decree --version` | Print the version |

decree never continues a run that was stopped by a signal or a crash: the run is `interrupted` until you run `decree retry`, because a kill may be deliberate.

## Files

```text
.decree/
  .gitignore                      # inbox/ and runs/
  machines/<name>.yml             # machines
  scripts/<name>                  # scripts shared by every machine
  scripts/<machine>/<name>        # a machine's own scripts, found first
  migrations/                     # ordered, run-once messages; committed, never edited
  processed.md                    # ledger of migrations that ran; committed
  inbox/                          # queued messages
  cron/                           # message templates queued on a schedule
  runs/<id>/                      # one folder per run: message.md, events.jsonl, logs
  graph/                          # written by decree graph; committed
```

There is no configuration file. What 0.4 configured is a convention or a fixed limit:

| Setting | Instead |
| --- | --- |
| Router for `choose: model` | The machine named `router`, unless the state sets `router:` |
| Default machine | None: every message names its `machine:` |
| Retries | `max_attempts` on the state; default 1 (no retry) |
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

`events.jsonl` is one JSON line per event, with `machine`, `state` and `run_id` on each, so any log shipper can read it. `mock/observability/config.alloy` ships it to Loki with Grafana Alloy, and `mock/README.md` has example queries.

## Docker

Run decree in a container with your AI tool installed on startup:

```yaml
services:
  decree:
    image: ghcr.io/jtmckay/decree:latest
    volumes:
      - .:/work
    environment:
      - DECREE_AI=opencode  # opencode, claude, or copilot
      - DECREE_DAEMON=true
      - DECREE_INTERVAL=2
    restart: unless-stopped
```

## Upgrading from 0.4

0.5.0 is a breaking release. `scripts/migrate-0.4-to-0.5.sh` moves a 0.4 project to the new layout: it moves pending inbox and outbox files into `inbox/`, moves the configuration file and other removed paths into `.decree/legacy-0.4/`, and lists the machines your pending messages ask for that you still have to write, and the messages that name no machine. Routines do not convert mechanically; write each as a machine plus scripts. Migrations are immutable, so messages still accept `routine:` as an alias of `machine:`.

| Removed in 0.5.0 | Instead |
| --- | --- |
| `decree routine`, `decree routine sync`, `decree routine verify` | Machines in `machines/`, validated by `decree check` |
| `decree cron list` | `decree status --cron` |
| `decree log` | `decree status <id>` |
| `decree skill` | `decree init` writes the skill |
| `.decree/routines/` | `.decree/machines/` and `.decree/scripts/` |
| `.decree/outbox/` | `decree emit` writes to `inbox/` |
| `dead/` folders | Runs end in a `failed` state; see `decree status` |
| `.decree/router.md` | A `choose: model` state in a machine (see `mock/.decree/machines/triage.yml`) |
| `.decree/prompts/` | Removed |
| `hooks` | `onentry` and `onexit` scripts (`git_baseline`, `snapshot`) |
| `routines`, `shared_routines`, `commands` | Removed; machines need no registry, and router machines call the AI tool |
| The configuration file (`default_routine`, `routine_source`, `max_retries`) | Conventions: `machine:` on every message, `max_attempts` on states, symlinks for sharing |
| `run.json` | `events.jsonl` |

## License

[MIT](LICENSE)
