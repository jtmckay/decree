# Tests

`cargo test` runs the unit tests in `src/` and the files below. Each test builds its own `.decree/` in a temp directory (or only reads this repository), drives the built `decree` binary, and calls no model and no network.

| File | Covers |
| --- | --- |
| `check_test.rs` | `decree check` on a copy of `mock/`: passes without a warning, graph warnings, and mock machines broken to hit a rule. |
| `cli_test.rs` | The CLI end to end: `init`, `emit`, `process`, `status`, `tail`, `retry`, `event` and `daemon`. |
| `config_test.rs` | No configuration file: a 0.4 `config.yml` fails every project command; a message without `machine:` is invalid. |
| `develop_test.rs` | The `develop` and `rust_develop` machines `init` writes, against 0.4.2's outcomes. |
| `docs_test.rs` | The docs hold together: links resolve, the machine examples are the `mock/` files. |
| `emit_test.rs` | `decree emit`: parent, depth, trigger, `emits`, `max_depth` and `--param`. |
| `examples_test.rs` | Every project in `examples/` checks, has no 0.4 term, and its README commands run. |
| `failure_test.rs` | Failure scenarios not covered elsewhere: SIGTERM then `decree retry`, two `process` at once, router replies rejected or below `min_confidence`, `max_depth`, `onexit` failures and the 2 MiB log cap. |
| `graph_test.rs` | `decree graph`: one file per machine, `system.md`, stale files, and the `mock/` graph byte for byte. |
| `hooks_test.rs` | `onentry`/`onexit` order, attempts, and `failed`'s `onentry` (0.4.2's hooks). |
| `integration_test.rs` | `decree init` (layout, routers, skill, permissions), `status`, color and exit codes. |
| `interpreter_props.rs` | Property tests (proptest): generated machines and script outcomes keep the interpreter's invariants (seq, transitions, visits, mirror, hook order). |
| `interrupt_test.rs` | Signals, crashes, the run lock and run status, including finished runs read from their last line. |
| `migrate_script_test.rs` | `scripts/migrate-0.4-to-0.5.sh` on the 0.4.2 project in `fixtures/legacy-0.4/`. |
| `mock_replay_test.rs` | Each `mock/` run that is not a router's child run, replayed through the binary with stub scripts: its events, child runs, `message.md` and `request.json` equal the mock's. |
| `mock_templates_test.rs` | Every `mock/` file that `init` also writes is byte-identical to it. |
| `process_test.rs` | `decree process`: inbox claim and validation, and the six migration rules. |
| `prune_test.rs` | `decree prune`: only finished runs older than the age, `--dry-run`, the runs it keeps (not finished, failed migration, child of an unfinished parent, locked), bad ages, and a pruned migration not run again. |
| `readme_test.rs` | `README.md` and `--help` describe 0.5 only; every README command runs. |
| `reply_test.rs` | Replies to waiting runs, `timeout_s`, and `decree event`. |
| `validation_test.rs` | Each validation rule V1–V21 and M1–M3: a passing and a failing case per rule, plus a failing case for each machine shape that migration 71 replaced (V19), as a table. |

`fixtures/` holds only what is not in `mock/`: the graph `system` project, the 0.4.2 project for the migrate script, the `step_*` machines and the scripts that unit tests in `src/` run, 0.4.2's routines for `develop_test.rs` (`scripts/v0_4_2/`), and the SCXML IRP notes.

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
