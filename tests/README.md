# Tests

`cargo test` runs the unit tests in `src/` and the files below. CI ([`.github/workflows/ci.yml`](../.github/workflows/ci.yml)) runs the same gate as `rust_develop`'s `gate` script, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test`, on every push to `main` and `v0.5` and every pull request; `ci_test.rs` keeps the two equal. Each test builds its own `.decree/` in a temp directory (or only reads this repository), drives the built `decree` binary, and calls no model and no network.

| File | Covers |
| --- | --- |
| `ci_test.rs` | `.github/workflows/ci.yml` parses as YAML, its `test` job runs exactly the gate script's three commands, and its `decree-check` job checks every project with `--format sarif` and uploads the results. |
| `check_test.rs` | `decree check` on copies of `examples/feature/` and `examples/sort-documents/`: passes without a warning, graph warnings, and example machines broken to hit a rule. |
| `cli_test.rs` | The CLI end to end: `init`, `emit`, `process` (and `process --retry`), `status`, `tail`, `event` and `daemon`, and one duration format for a machine's `timeout`, `prune` and `daemon`. |
| `develop_test.rs` | The `develop` and `rust_develop` machines `init` writes: their outcomes, QA, `STOP`, sessions and usage limits. |
| `docs_test.rs` | The docs hold together: links resolve, the machine examples are the `examples/feature/` files, and `CHANGELOG.md` has its 0.5.0 entry and links the versioning rule. |
| `emit_test.rs` | `decree emit`: parent, depth, trigger, `emits`, `max_depth` and `--param`, and the `traceparent` it copies from a script's `TRACEPARENT`. |
| `examples_test.rs` | Every project in `examples/` checks and ships its graph and schema, and its README's `bash` blocks run (other fences, such as the GLiNER quick start's `sh`, are for a real machine); no word from the previous major version (listed in the test) remains in `src/`, `docs/reference/`, `README.md`, `tests/` or `examples/`. |
| `failure_test.rs` | Failure scenarios not covered elsewhere: SIGTERM then `decree process --retry`, two `process` at once, router replies rejected or below `min_confidence`, `max_depth`, `onexit` failures and the 2 MiB log cap. |
| `format_test.rs` | `--format json` on `check`, `graph`, `schema`, `emit`, `event`, `status` (overview, an active run, one run, an unknown id), `prune` and `process --dry-run`: each document validates against its schema in `.decree/schema/v1/cli/`, and the exit code equals text mode's (run on a copy of the project), on success and on each error; `--format` where it does not apply exits 2. |
| `graph_test.rs` | `decree graph`: one file per machine, `system.md`, stale files, and every example's graph byte for byte. |
| `hooks_test.rs` | `onentry`/`onexit` order, attempts, and `failed`'s `onentry`. |
| `integration_test.rs` | `decree init` (layout, routers, skill, permissions), `status`, color and exit codes. |
| `interpreter_props.rs` | Property tests (proptest): generated machines and script outcomes keep the interpreter's invariants (seq, transitions, visits, mirror, hook order), and every line they write validates against `events.schema.json` (compiled once), has the run's one `trace_id`, names a `span_id` no other event names, and agrees with `traces.jsonl`. |
| `interrupt_test.rs` | Signals, crashes, the run lock and run status, including finished runs read from their last line. |
| `process_test.rs` | `decree process`: inbox claim and validation, messages without `machine:` (M1–M3), and the six migration rules. |
| `prune_test.rs` | `decree prune`: only finished runs older than the age, `--dry-run`, the runs it keeps (not finished, failed migration, child of an unfinished parent, locked), bad ages, and a pruned migration not run again. |
| `schema_test.rs` | The JSON Schemas in `.decree/schema/v1/`: each valid draft 2020-12 with a description on every property; every machine in `examples/`, the templates, this repository and a fresh `init` validates, and every message, `events.jsonl` line, `request.json` and `reply.json` in `examples/`, and every recorded `traces.jsonl` agrees with its events (OTLP/JSON shape checked in `common/traces.rs`, without a schema); a `model` run made in the test writes a request, a reply and events that validate; wrong events and replies are rejected; `decree schema` writes `v1/` and `v1/cli/` and removes the unversioned files, and `check` warns until it has. |
| `readme_test.rs` | `README.md` and `--help` cover every command and name no removed concept; every README command runs. |
| `replay_test.rs` | Each recorded run in `examples/` that is not a router's child run, replayed through the binary with stub scripts that replay each recorded log, exit code and named event: its events, child runs, `message.md`, `request.json` (with its `reply_schema`) and `traces.jsonl` equal the recorded ones, with generated run, trace and span ids mapped to the recorded ones. Every such run must have a test. Every recorded `reply.json` validates against its request's `reply_schema`, whose `event` enum is the options. |
| `route_by_complexity_test.rs` | `examples/route-by-complexity/`'s `develop_by_size` through the binary, with a stub `ask_gliner.sh` and stub `opencode`, `claude` and test commands: `small`, `large`, `unsure`, a local attempt that fails verification and escalates to Claude, and both failing. The classifier's input holds the named files' line counts, the request's `reply_schema` allows exactly the options, the GLiNER quick starts are at most five commands, the server code is in one file, and `decide_server.py` byte-compiles (skipped without `python3`). |
| `reply_test.rs` | Replies to waiting runs, `timeout`, and `decree event`. |
| `trace_test.rs` | Traces (W3C Trace Context, OTLP/JSON): a run, its router run and its child machine run share one trace id with the documented parent spans and a span for each run, script and decision; an inbox message's `traceparent` is honoured and an invalid one ignored, giving a new trace; a script's `TRACEPARENT` names its own span (and an inherited one is replaced); failed scripts and runs are `ERROR`; an interrupted run's span is written on recovery (or with the signal) with `ERROR`, and `decree process --retry` links a new run span. |
| `templates_test.rs` | Every example file that `init` also writes (the git scripts, and the `router` copy in each recorded example) is byte-identical to it, and the three `gliner_router` copies are identical. |
| `tmux_services_test.rs` | `examples/tmux-services/`'s `illustrated_post` through the binary, with stub `tmux` (sessions as files, calls logged), `curl` (health URLs answer while the stub session exists; requests logged; loaded models as files that ComfyUI's `/prompt` and `/free` and Ollama's `/api/generate` set and clear, reported by `/system_stats` and `/api/ps`) and `ask_gliner.sh`: `with_picture` starts each session once and kills none, and unloads ComfyUI through `/free` before Ollama's request, `without_ollama` unloads a loaded model with `keep_alive: 0`, a service that already answers starts no session, an unload that never takes effect or a failed or lost prompt fails the state, and `without_comfy_no_wait` clears and interrupts before `/free`. |
| `validation_test.rs` | Each validation rule V1–V21 and M1–M3: a passing and a failing case per rule, plus a failing case for each machine shape that migration 71 replaced (V19), as a table. The same cases hold the JSON Schemas to `decree check`: `CHECK_ONLY` lists, with the reason, each failing file only `decree check` can catch. On every case `decree check --format json` validates and lists the same errors as the text, and on every failing case `--format sarif` has every rule and a result per error with its `ruleId`, `uri` and `startLine`. |

`fixtures/` holds only what is not in `examples/`: the graph `system` project, the `step_*` machines and the scripts that unit tests in `src/` run, and the SCXML IRP notes.

A test that runs a script it wrote must write it from a child process (`sh -c 'cat > "$1"'`, or `cp`), never with `fs::write`: tests run on parallel threads, a process another thread forks inherits the test process's open write handle until it execs, and running the script meanwhile fails with ETXTBSY ("Text file busy").

## Adding a validation case

Add a `Case` to `CASES` in `validation_test.rs`:

```rust
Case {
    rule: "V4",
    name: "a target that does not exist",
    files: &[("machines/m.yml", "name: m\n...")],
    scripts: &[],
    expected: "machines/m.yml: work: transition `skip` targets unknown state `nowhere` (V4)\n",
},
```

- `files` are written under `.decree/`: machines, and any `migrations/`, `inbox/`, `cron/` or `processed.md`.
- Every script a machine names (`invoke`, `script`, `ask`, `onentry`, `onexit`) gets an executable stub at `scripts/<name>`. List a script in `scripts` to change that: `Script::Missing("setup")`, `Script::At("m/snapshot.sh")` or `Script::NotExecutable("snapshot.sh")`.
- `expected` is the exact stdout of `decree check`, or `PASSES` for exit 0 and no output. Each line of a failing case must end with `(<rule>)`.
- A new rule also needs a line in `RULES` and in `rule_tests!`; `every_rule_has_a_passing_and_a_failing_case` fails until it has both.
- `schemas_agree_with_decree_check_on_every_case` validates the case's machines and messages against the JSON Schemas: a passing case must validate, and every file a failing case reports must be rejected, unless the case's file is in `CHECK_ONLY` with the reason only `decree check` catches it.
