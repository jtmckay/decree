# decree 0.5 reference

decree runs durable state-machine workflows from plain files: every run is an append-only log that survives crashes and restarts and continues where it stopped. It needs no AI; when a workflow uses AI, models and people only pick among the options a state declares. decree is built from three building blocks: **messages** (markdown), **machines** (YAML statecharts) and **scripts** (executables, bash by default). This reference describes how each behaves. A small project with real files, frozen partway through its runs, lives in [`examples/project/`](../../examples/project/README.md) (three simple machines, a finished run, a waiting run and an interrupted run). Why decree works this way is in [the decision log](../decisions.md).

| File | Subject |
| --- | --- |
| [messages.md](messages.md) | Frontmatter, parsing, the lifecycle of a message, replies, the run lock, migrations, cron files |
| [machines.md](machines.md) | Examples, `invoke`, keys, the store, the SCXML subset, rules, the machine schema, validation V1–V21, M1–M3 and E1 |
| [scripts.md](scripts.md) | Resolution, execution, environment (`.decree/env`, invoke `env`), shared code in `lib/`, the store, run-directory files, events from an invoke |
| [runs.md](runs.md) | The step loop, `check`, `model` and routers, sub-machines, `person`, `events.jsonl` |
| [cli.md](cli.md) | Every command and its exit codes |
| [graph.md](graph.md) | `decree graph`: the Mermaid documents and how to view them |
| [observability.md](observability.md) | Shipping `events.jsonl` and script logs to Loki; traces: W3C Trace Context, `TRACEPARENT` and `traces.jsonl` for the OpenTelemetry Collector |
| [standards.md](standards.md) | The standards and prior art decree draws from, and where it deviates |

Related guides: [routers](../routers.md) (router machines for other models) and [services](../services.md) (long-running services next to decree). What changed between releases is in the [changelog](../../CHANGELOG.md); the files below are versioned as [Versioning](#versioning) says.

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
  |      | script name + DECREE_* env       ^ event: exit code or event file
  |      v                                  |
  +-- Scripts   (scripts/<name>, bash by default)
```

Everything that crosses a boundary is listed below. Nothing else crosses.

| From → to | What crosses, exactly | Reference |
| --- | --- | --- |
| Message → machine | Frontmatter `machine` and `params`. The body is passed through untouched. | [messages.md](messages.md) |
| Machine → script | A script name, plus the `DECREE_*` environment variables. | [scripts.md](scripts.md) |
| Script → machine | One event, from an invoke only: from the exit code, or the name it writes to `$DECREE_EVENT_FILE`. | [scripts.md](scripts.md#events-from-an-invoke) |
| Machine → model → machine | A `model` state's options with their descriptions, its `output` state's output and the message body. One event back, validated against the options. | [runs.md](runs.md#model) |
| Person → machine | A reply message naming the wait id and one of the options of a `person` state. | [messages.md](messages.md#replies) |
| Script → message | `decree emit` writes a new message to `inbox/`. Allowed only for machines in the state's `emits`. | [cli.md](cli.md) |

Three guardrails keep the blocks apart:

- Machines contain no code and no file paths.
- Scripts make no routing decisions beyond an invoke naming one event.
- Messages hold no graph data. decree mirrors the current `state` into the run's copy for humans; the run's `events.jsonl` is the record.

## File layout

Each building block has its own directory in `.decree/`:

```text
.decree/
  .gitignore                        # contains: inbox/, runs/ and store/
  migrations/                       # ordered run-once messages, committed, never edited (messages.md)
  processed.md                      # committed ledger: one migration filename per line
  inbox/                            # queued messages: *.md; files starting with "." are ignored
  runs/<message id>/                # one folder per run, created when the message is claimed
    message.md                      # the claimed message; decree mirrors frontmatter `state`
    events.jsonl                    # one JSON line per event: the run's record and its telemetry (runs.md)
    traces.jsonl                    # one OTLP/JSON span per line, as each span ends (observability.md, Traces)
    0001-<state>-<script>.log       # stdout+stderr of each script execution, numbered in run order
    .lock                           # pid of the process stepping this run (messages.md, Run lock)
    .running                        # the script running now: pid, state, phase, script, started_at, log (scripts.md)
    .event                          # the running invoke's DECREE_EVENT_FILE, deleted once read (scripts.md)
  cron/                             # *.md cron templates (messages.md, Cron files)
  machines/<machine name>.yml         # statecharts (machines.md)
  graph/<machine name>.md, system.md  # written by `decree graph`; committed, so graphs render on GitHub (graph.md)
  schema/v1/*.schema.json             # written by `decree schema`; committed, so editors check machines as you type (Schemas, below)
  scripts/<name>                    # executables shared by every machine; optional extension: verify.sh (scripts.md)
  scripts/<machine name>/<name>       # optional: a machine's own script, overriding scripts/<name> for that machine
  lib/                              # code that scripts source, config and data: $DECREE_LIB; decree never runs anything in it (scripts.md, Shared code); `decree init` writes `ai.sh`
  env                               # optional, committed: KEY=value variables every script gets, no secrets (scripts.md, Environment)
  store/<machine name>/             # what the machine keeps between runs: $DECREE_STORE, declared under `store:`; never deleted by decree (scripts.md, Store)
```

`migrations/` and `processed.md` are committed to git; `inbox/`, `runs/` and `store/` are not. A run folder may also hold `received/` (delivered replies, [Replies](messages.md#replies)), `request.json` and `reply.json` (in a router run, [Model](runs.md#model)).

### No configuration file

A project is machines, scripts and messages; there is no configuration file for decree. (`.decree/env` holds variables for scripts; decree reads no setting of its own from it.) Each setting is a convention or a built-in limit:

| Setting | Instead |
| --- | --- |
| Router for `model` | A `model` with no `router:` uses the machine named `router` ([Model](runs.md#model)). `decree init` writes it. |
| Default machine | None: every message names its machine with `machine:`. `decree emit`, cron files and `decree init`'s examples always do. |
| Retries | `attempts` in the script invoke, a number or a list of values; default 1 (no retry), as a Step Functions task without `Retry`. |
| Emit depth | A fixed limit of 10 (`max_depth`). |
| Log size | Each script log is capped at 2 MiB (2097152 bytes). |
| Sharing across projects | None in decree. To share machines or scripts across projects, symlink them into `machines/` and `scripts/`. |

The daemon poll interval is the `decree daemon --interval` flag.

## Schemas

Every file decree reads or writes, and every document a command prints with `--format json`, has a JSON Schema (draft 2020-12). `decree schema` writes them into `.decree/schema/v1/` and removes anything else in `.decree/schema/`, `decree init` writes them, and `decree check` warns when one is missing, out of date, or joined by a file decree does not write (such as the unversioned `schema/machine.schema.json` of earlier 0.5 builds). Their single source is [`src/templates/schema/v1/`](../../src/templates/schema/v1/), compiled into decree; each schema's `$id` is its raw URL in the decree repository on GitHub. Every property has a description taken from this reference.

| Schema | Covers | Documented in |
| --- | --- | --- |
| `machine.schema.json` | A machine file, `machines/<name>.yml`. Machines point editors at it ([Schema](machines.md#schema)). | [machines.md](machines.md) |
| `message.schema.json` | The frontmatter of a migration, inbox message, cron file or reply. | [messages.md](messages.md#frontmatter-keys) |
| `events.schema.json` | One line of `runs/<id>/events.jsonl`: the common fields, then one branch per `type`. | [runs.md](runs.md#eventsjsonl) |
| `request.schema.json` | A router run's `request.json`, including `reply_schema`. | [runs.md](runs.md#model) |
| `reply.schema.json` | The general shape of a router's `reply.json`. The `reply_schema` in each request stays the exact schema for that request. | [runs.md](runs.md#model) |
| `cli/<command>.schema.json` | The document `decree <command> --format json` prints, for `check`, `status`, `emit`, `event`, `prune`, `graph`, `schema` and `process` (`--dry-run`). `status`'s `events` refer to `events.schema.json`. | [cli.md](cli.md#machine-readable-output) |

The schemas describe shape; `decree check` stays the authority for meaning. The events, request and reply schemas list every field decree writes and allow no other, so decree's tests catch a field the reference does not document. A consumer should still ignore fields and event types it does not know, because `v1` may gain them.

## Versioning

The files above are decree's contract with editors, routers, dashboards and pipelines. Its version is in the schema path, `v1`, and in the `v` field of every event and request. The rule follows Semantic Versioning 2.0.0, with the version in the path as Kubernetes API groups do (`apps/v1`):

- Within `v1`, changes are additive only: new optional fields, new event types, new optional keys.
- A rename, a removal, or a change of meaning is `v2`: a new directory, `.decree/schema/v2/`, and `v: 2` in events.
- decree 0.x may still change `v1` before 1.0 (SemVer item 4: major version zero is for initial development), and every such change is listed in the [changelog](../../CHANGELOG.md).
