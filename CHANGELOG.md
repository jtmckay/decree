# Changelog

All notable changes to decree are listed here. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/), and decree follows [Semantic Versioning 2.0.0](https://semver.org/spec/v2.0.0.html). The files decree reads and writes are versioned separately, in their schema path (`v1`), as [Versioning](docs/reference/README.md#versioning) states: within `v1` changes are additive only, and while decree is 0.x every change to `v1` is listed here.

## [0.5.0] - Unreleased

decree 0.5 is a rewrite. A project is three building blocks: **messages** (markdown with YAML frontmatter), **machines** (YAML statecharts that follow W3C SCXML) and **scripts** (executables). A message names its machine, the machine names scripts and decisions, and every run is recorded in `.decree/runs/<id>/events.jsonl`. A `model` decision is answered by a router machine through `request.json` and `reply.json`, and a `person` decision by a reply message. The contract is described in the [reference](docs/reference/README.md), with the reasoning in the [decision log](docs/decisions.md). There is no upgrade path from 0.4.

The entries below are the changes made to the 0.5 contract since migration 70 (`decree prune`), one per migration. Each is described in its decision log entry.

### Added

- A JSON Schema (draft 2020-12) for machines and message frontmatter, written by `decree schema` and by `decree init`. Every machine starts with a `# yaml-language-server: $schema=…` line, so editors complete keys and underline mistakes; `decree check` warns when the schemas are missing or stale (migration 72, [D45](docs/decisions.md#d45-json-schema-for-shape-decree-check-for-meaning)).
- `reply_schema` in `request.json`: the exact JSON Schema of the reply, which a typed router hands to a constrained decoder. Docs and examples for typed routers (GLiNER2.5-Decide, Ollama with `format`) and untyped ones, and the `route-by-complexity` example (migration 74, [D47](docs/decisions.md#d47-typed-routers-for-routing-untyped-models-for-the-work)).
- The `tmux-services` example: services in tmux sessions, with GLiNER beside ComfyUI and Ollama sharing a GPU (migration 75). Its ComfyUI jobs run in the background, and the queue is drained only before ComfyUI's GPU memory is needed (migration 76).
- A versioned schema for every file decree reads or writes: `events.schema.json` (one line of `events.jsonl`), `request.schema.json` and `reply.schema.json`, beside the machine and message schemas, all in `.decree/schema/v1/`. The versioning rule is in [Versioning](docs/reference/README.md#versioning). Tests validate every recorded event, request and reply, and every line the property test writes (migration 81, [D50](docs/decisions.md#d50-a-versioned-schema-for-every-file)).
- This changelog (migration 81).
- `--format json` on `check`, `status`, `emit`, `event`, `retry`, `prune`, `graph`, `schema` and `process --dry-run`: one JSON document on stdout, described by a new schema in `.decree/schema/v1/cli/<command>.schema.json`, with exit codes unchanged; and `decree check --format sarif`, a SARIF 2.1.0 log for GitHub code scanning, GitLab and Azure DevOps (migration 82, [D51](docs/decisions.md#d51-json-output-for-every-report-and-sarif-for-decree-check)).

### Changed

- **Breaking (machines):** every machine key has one shape. `invoke` is a map with one key naming its kind (`script`, `check`, `model`, `person`, `machine`), a check reads a named `output` state's output, its events are `true` and `false`, and function settings sit inside `invoke`. Old shapes fail `decree check` (V19) with a message naming the new one (migration 71, [D44](docs/decisions.md#d44-one-shape-per-machine-key)).
- `mock/` became examples by topic: `examples/feature/`, `examples/sort-documents/` and `examples/observability/` (migration 73, [D46](docs/decisions.md#d46-no-04-compatibility-and-examples-by-topic)).
- `tmux-services` scripts are named for what they do: `use_<service>` starts or uses a service, `without_<service>` frees the GPU of it (migration 77). They keep both servers up and free GPU memory by unloading models through the Ollama and ComfyUI APIs, instead of ending sessions (migration 78).
- **Breaking (scripts):** a script names its event by writing it to `$DECREE_EVENT_FILE`, not by printing `{"event": …}` last on stdout. In `events.jsonl`, its `transition` source is `script` (was `stdout`) (migration 79, [D48](docs/decisions.md#d48-scripts-name-their-event-in-decree_event_file)).
- **Breaking (machines and CLI):** one duration format everywhere, a whole number and `s`, `m`, `h` or `d`: a machine's `timeout` (was `timeout_s` in seconds), `decree prune --older-than` and `decree daemon --interval` (migration 80, [D49](docs/decisions.md#d49-one-duration-format)).
- **Breaking (schema location):** the schemas moved from `.decree/schema/` to `.decree/schema/v1/`, and machines point at `../schema/v1/machine.schema.json`. `decree check` warns about any other file in `.decree/schema/`, and `decree schema` removes it (migration 81).
- `events.jsonl`: a child run's claim event leaves `file` out instead of writing `null`, since a child run has no file (migration 81).

### Removed

- Everything 0.4: the upgrade script and its fixtures, the `config.yml` error, and the `routine:` alias for `machine:` (migration 73, [D46](docs/decisions.md#d46-no-04-compatibility-and-examples-by-topic)).

### Fixed

- `docs/reference/runs.md` documents the fields of `run_finished` again, the fields of each kind of `waiting` event, and when a `model` decision has no `child_run` or `pick` (migration 81).

[0.5.0]: https://github.com/jtmckay/decree/tree/v0.5
