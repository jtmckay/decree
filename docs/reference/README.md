# decree 0.5 reference

decree is built from three building blocks: **messages** (markdown), **machines** (YAML statecharts) and **scripts** (executables, bash by default). This reference describes how each behaves. A worked example of everything here, with real files, lives in [`mock/`](../../mock/README.md). Why decree works this way is in [the decision log](../decisions.md).

| File | Subject |
| --- | --- |
| [messages.md](messages.md) | Frontmatter, parsing, the lifecycle of a message, replies, the run lock, migrations, cron files |
| [machines.md](machines.md) | Examples, `invoke`, keys, the SCXML subset, rules, validation V1–V21 and M1–M3 |
| [scripts.md](scripts.md) | Resolution, execution, environment, events from an invoke |
| [runs.md](runs.md) | The step loop, `check`, `model` and routers, sub-machines, `person`, `events.jsonl` |
| [cli.md](cli.md) | Every command and its exit codes |
| [graph.md](graph.md) | `decree graph`: the Mermaid documents and how to view them |
| [observability.md](observability.md) | Shipping `events.jsonl` and script logs to Loki |
| [standards.md](standards.md) | The standards and prior art decree draws from, and where it deviates |

Related guides: [routers](../routers.md) (router machines for other models) and [services](../services.md) (long-running services next to decree).

## Terms

Machines follow W3C SCXML 1.0 (the State Chart XML recommendation): its terms, its transition semantics, and a strict subset of its features ([SCXML subset](machines.md#scxml-subset)). They are written in YAML, never XML. Anyone, human or AI, who knows SCXML knows how a decree machine behaves.

| Term | Meaning |
| --- | --- |
| Message | A markdown file: YAML frontmatter (structured) plus a body (unstructured task text). One message = one run. |
| Machine | A YAML statechart in `.decree/machines/<name>.yml`: one SCXML document, written as YAML. A type, never an instance. |
| State | An SCXML state: atomic (does one step), compound (has child `states`) or final (`final: true`, ends the run). |
| Configuration | SCXML's term for the set of active states: the current atomic state and all its ancestors. |
| Script | An executable file in `.decree/scripts/` (shared by all machines) or `.decree/scripts/<machine>/` (one machine's override). Machines refer to scripts by name, never by path. |
| Invoke | A state's function, `invoke:` with one key naming its kind (`script`, `check`, `model`, `person` or `machine`); `invoke: <name>` runs a script. An SCXML `<invoke>`: it runs while the state is active and its completion raises exactly one event. |
| `onentry`, `onexit` | Scripts run when a state is entered or exited: SCXML executable content in `<onentry>` and `<onexit>`. They produce no event. |
| Event | A name such as `done`, `error` or `pass` that selects a transition out of a state. |
| Transition | An entry under a state's `transitions:` mapping an event to a `target` state: an SCXML `<transition event target>`. |
| Data | A machine's typed, read-only values (`data:`), set from a message's `params`: SCXML `<data>`, initialised the way `<invoke><param>` initialises an invoked session. |
| Decision | A built-in function a state can invoke instead of a script: `check` (a deterministic condition), `model` (a router machine asks a model to pick an option) or `person` (a person picks an option). See [Invoke](machines.md#invoke-the-states-function). |
| Sub-machine | A machine a state invokes, `invoke: { machine: <name> }`: SCXML's nested session. It runs as a child run with its own folder and a `parent` reference; the final state it reaches is the parent state's event. |
| Router | A machine that answers a `model` request: it reads the request, asks a model however it likes, and writes a reply. Replaceable; `decree init` writes one named `router`. |
| Run | One message moving through one machine (an SCXML session), stored in `.decree/runs/<message id>/`. |

## Architecture

decree splits structured from unstructured data across three blocks. Messages carry the task, machines carry the control flow, and scripts do the work. The LLM only ever chooses among edges a machine declares.

```text
            decree emit
  +-------------------------------+
  |                               v
  |   Message   (frontmatter = structured, body = unstructured)
  |      | names machine; decree mirrors state
  |      v
  |   Machine   (states, invokes, transitions)            <-- options / one event -->  model or person
  |      | script name + DECREE_* env       ^ event: exit code or JSON line
  |      v                                  |
  +-- Scripts   (scripts/<name>, bash by default)
```

Everything that crosses a boundary is listed below. Nothing else crosses.

| From → to | What crosses, exactly | Reference |
| --- | --- | --- |
| Message → machine | Frontmatter `machine` and `params`. The body is passed through untouched. | [messages.md](messages.md) |
| Machine → script | A script name, plus the `DECREE_*` environment variables. | [scripts.md](scripts.md) |
| Script → machine | One event, from an invoke only: from the exit code, or a JSON object on the last stdout line. | [scripts.md](scripts.md#events-from-an-invoke) |
| Machine → model → machine | A `model` state's options with their descriptions, its `output` state's output and the message body. One event back, validated against the options. | [runs.md](runs.md#model) |
| Person → machine | A reply message naming the wait id and one of the options of a `person` state. | [messages.md](messages.md#replies) |
| Script → message | `decree emit` writes a new message to `inbox/`. Allowed only for machines in the state's `emits`. | [cli.md](cli.md) |

Three guardrails keep the blocks apart:

- Machines contain no code and no file paths.
- Scripts make no routing decisions beyond an invoke printing one event.
- Messages hold no graph data. decree mirrors the current `state` into the run's copy for humans; the run's `events.jsonl` is the record.

## File layout

Each building block has its own directory in `.decree/`:

```text
.decree/
  .gitignore                        # contains: inbox/ and runs/
  migrations/                       # ordered run-once messages, committed, never edited (messages.md)
  processed.md                      # committed ledger: one migration filename per line
  inbox/                            # queued messages: *.md; files starting with "." are ignored
  runs/<message id>/                # one folder per run, created when the message is claimed
    message.md                      # the claimed message; decree mirrors frontmatter `state`
    events.jsonl                    # one JSON line per event: the run's record and its telemetry (runs.md)
    0001-<state>-<script>.log       # stdout+stderr of each script execution, numbered in run order
    .lock                           # pid of the process stepping this run (messages.md, Run lock)
    .running                        # the script running now: pid, state, phase, script, started_at, log (scripts.md)
  cron/                             # *.md cron templates (messages.md, Cron files)
  machines/<machine name>.yml         # statecharts (machines.md)
  graph/<machine name>.md, system.md  # written by `decree graph`; committed, so graphs render on GitHub (graph.md)
  scripts/<name>                    # executables shared by every machine; optional extension: verify.sh (scripts.md)
  scripts/<machine name>/<name>       # optional: a machine's own script, overriding scripts/<name> for that machine
```

`migrations/` and `processed.md` are committed to git; `inbox/` and `runs/` are not. A run folder may also hold `received/` (delivered replies, [Replies](messages.md#replies)), `request.json` and `reply.json` (in a router run, [Model](runs.md#model)).

decree 0.4's `outbox/`, `dead/`, `router.md`, `routines/`, `prompts/` and `config.yml` do not exist in 0.5.

### No configuration file

A project is machines, scripts and messages; there is no `config.yml`. What 0.4 configured is a convention or a built-in limit:

| 0.4 setting | 0.5 |
| --- | --- |
| Router for `model` | A `model` with no `router:` uses the machine named `router` ([Model](runs.md#model)). `decree init` writes it. |
| Default routine | None: every message names its machine with `machine:` (or the `routine:` alias). `decree emit`, cron files and `decree init`'s examples always do. |
| `max_retries` | `max_attempts` in the script invoke; default 1 (no retry), as a Step Functions task without `Retry`. |
| Emit depth | A fixed limit of 10 (`max_depth`). |
| Log size | Each script log is capped at 2 MiB (2097152 bytes). |
| Shared routines | None in decree. To share machines or scripts across projects, symlink them into `machines/` and `scripts/`. |

The daemon poll interval is the `decree daemon --interval` flag. A `.decree/config.yml` left from 0.4 is an error that names `scripts/migrate-0.4-to-0.5.sh`: `.decree/config.yml is not used by decree 0.5; run scripts/migrate-0.4-to-0.5.sh`.
