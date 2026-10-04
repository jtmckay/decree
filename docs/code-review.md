# Code review of `src/` (0.5)

A review of `src/` after the 0.5 rewrite, which fourteen migrations built and several reworked. Each finding below is **fixed** (behaviour unchanged, the black-box tests in `tests/` are the safety net) or **kept**, with the reason. Lines of fixed findings refer to the tree before the review (commit `faedf99`); lines of kept findings refer to the tree after it.

## Modules

Non-test lines (`#[cfg(test)]` items and `tests.rs` files excluded), before and after.

| Module | Job | Before | After |
| --- | --- | ---: | ---: |
| `lib.rs`, `main.rs` | Parse the command line and dispatch to a command. | 74 | 62 |
| `cli.rs` | The clap command line. | 128 | 128 |
| `error.rs` | `DecreeError`, exit codes, finding the project root. | 83 | 89 |
| `layout.rs` | Names of the `.decree/` and run-folder entries. | 9 | 15 |
| `message.rs` | Messages: parse, write, id, claim, queue, run lock, run folders, frontmatter checks. | 584 | 624 |
| `events.rs` | `events.jsonl`: append, read, and what is derived from it (state, visits, confidences). | new | 191 |
| `machine.rs` | Machines: YAML types, parsing with V19 messages, the arena and its queries. | 1684 | 717 |
| `machine/validate.rs` | Validation rules V1–V21 on the arena. | (in `machine.rs`) | 992 |
| `cond.rs` | `check` conditions: shape (V10) and evaluation. | 460 | 446 |
| `runtime.rs` | The executor: running a script with its environment, log, process group, timeout and attempts. | 929 | 705 |
| `runtime/resolve.rs` | Script resolution (V12 and the executor). | (in `runtime.rs`) | 155 |
| `interpreter.rs` | The step loop: entry, invoke, transition selection, exit, record, finish. | 1728 | 792 |
| `interpreter/decide.rs` | Decision invokes: `check`, `choose: model` (request, reply), `choose: person`. | (in `interpreter.rs`) | 457 |
| `interpreter/child.rs` | Child runs: `machine` invokes, router runs, continuing a parent. | (in `interpreter.rs`) | 178 |
| `interpreter/recover.rs` | Run status, crash recovery, the `state` mirror, invalid messages. | (in `interpreter.rs`) | 233 |
| `reply.rs` | Replies to `choose: person` waits: check, deliver, timeouts. | 286 | 245 |
| `graph.rs` | Mermaid documents from the arena. | 302 | 290 |
| `cron.rs` | Cron files: scan, schedule, due check, the message a firing queues. | 167 | 167 |
| `commands/*.rs` | One file per command (`check` also loads a `Project`, `process` holds the pipeline `daemon` shares). | 2416 | 2276 |
| **Total** | | **8850** | **8762** |

`machine/validate.rs` is the only module over 800 lines, and it has one job.

## Findings

### Logic that existed twice

| # | Where | Finding | Status |
| --- | --- | --- | --- |
| 1 | `src/graph.rs:205`, `src/interpreter.rs:1544` | The transition domain was computed by two functions with different code. | Fixed: one `LoadedMachine::transition_domain`; the `mock/` graph is still byte for byte. |
| 2 | `src/interpreter.rs:1505` | `is_root_final` was a free function next to the arena's own queries. | Fixed: `LoadedMachine::is_root_final`. |
| 3 | `src/interpreter.rs:851`, `src/interpreter.rs:1207` | "Has this child run finished, and in which state" was worked out twice (`child_final`, and inside `Context::status`). | Fixed: `Context::final_state`, used by both. |
| 4 | `src/commands/process.rs:307`, `:549`, `src/interpreter.rs:1331`, `src/reply.rs:223` | Listing run folders in `id` order was written four times. | Fixed: `message::run_ids`. |
| 5 | `src/commands/process.rs:322`, `src/interpreter.rs:854`, `:1225`, `:1352`, `src/reply.rs:80`, `src/commands/retry.rs:33` | "The machine a run's first event names" was written six times. | Fixed: `Context::run_machine` and `events::first_text`. |
| 6 | `src/commands/process.rs:290`, `:155` | Status for display (`run_status`) and the `pending` scan each read events and the lock. | Fixed: `Context::status_of`, used by `pending`, `status` and `tail`. |
| 7 | `src/commands/process.rs:385`, `:422`, `src/interpreter.rs:1253` | `machine`, else `routine`, read from frontmatter three times. | Fixed: `Message::machine`. |
| 8 | `src/commands/process.rs:430`, `src/interpreter.rs:1269` | `params` and `depth` read from frontmatter, and `RunInput` built, twice. | Fixed: `Message::params`, `Message::depth`, `RunInput::new`. |
| 9 | `src/interpreter.rs:304`, `:340` | Entering from the root after a claim and after `decree retry` were two copies. | Fixed: `Interpreter::enter_from_root`. |
| 10 | `src/interpreter.rs:1110`, `:1371`, `:1697`, `src/reply.rs:282`, `src/commands/retry.rs:89`, `src/runtime.rs:590`, `:696` | Each caller of `EventLog::append` turned a `json!` object into a `Map`, five of them behind an `unreachable!`. | Fixed: `EventLog::append` takes the JSON object and returns an error for anything else. |
| 11 | across `src/` | `e.get(key).and_then(Value::as_str)` and the same for `type`, `options`, `child` were repeated about 40 times. | Fixed: `events::{text, is_type, strings, waiting_child, claim_event}`. |
| 12 | `src/reply.rs:19`, `src/interpreter.rs:90` | `ReplyError` was a copy of `InterpreterError::Io` with its own `io_err`. | Fixed: replies use `InterpreterError`; `reply` takes the `Context`. |
| 13 | `src/commands/process.rs:636`, `src/commands/emit.rs:161`, `src/commands/event.rs:41` | Three copies of "any error to `DecreeError::Other`". | Fixed: `DecreeError` converts `MessageError` and `InterpreterError` with `#[from]` (same message, same exit code). |
| 14 | `src/cond.rs:161`, `:438` | Int comparison and confidence comparison were two copies of the operator table. | Fixed: one generic `Op::compare`. |
| 15 | `src/message.rs:546`, `src/cron.rs:60` | A second frontmatter parser, `parse_frontmatter`, from 0.4, used only by cron files. It differs from `Message::parse`: no BOM or CRLF, an unclosed fence is body, keys come back sorted. `decree check` (`src/commands/check.rs:186`) and `decree graph` (`src/commands/graph.rs:94`) read cron files with `Message::parse`, so a CRLF cron file passes `check` but never fires, and a fired message's keys are sorted, not as written. | **Fixed** after review, by decision: cron files are parsed with `Message::parse`; a fired message keeps the cron file's keys in the order written, then `trigger: cron`. `parse_frontmatter` is deleted. |
| 16 | `src/commands/tail.rs:180`, `src/runtime.rs:550` | Two parsers of the `NNNN-` log number: tail wants exactly 4 digits, the executor 4 or more, so `tail` would skip logs from 10000 on. | **Fixed** after review, by decision: `tail` accepts 4 or more digits, as the executor writes them; the reference docs say so too. |
| 17 | `src/runtime.rs:97`, `src/interpreter.rs:748` | `data_env` (strings for `DECREE_DATA_*`) and `data_values` (typed values for `check`) both resolve `params` over defaults. | Kept: two outputs for two consumers; `data_env` cannot fail while `data_values` can, and the executor is built before the interpreter. |
| 18 | `src/commands/graph.rs:88`, `src/cron.rs:59` | `decree graph` reads a cron file's machine itself instead of through `cron.rs`. | Kept: tied to finding 15; `cron.rs` skips a cron file without `cron:`, where `graph` must still fail on a missing machine. |

### Functions, types or fields with one caller or only tests

| # | Where | Finding | Status |
| --- | --- | --- | --- |
| 19 | `src/machine.rs:379`, `:386`, `:406` | `machine_paths` wrapped `machine_files`; `load_machine_file` had one caller. | Fixed: one `machine_paths`, loading inlined. |
| 20 | `src/runtime.rs:530` | `Executor::max_attempts` did not use the executor. | Fixed: `LoadedMachine::max_attempts`. |
| 21 | `src/commands/process.rs:269` | `Pipeline::print_waiting` only called `print_waiting`. | Fixed: removed. |
| 22 | `src/commands/status.rs:307` | `--cron` sorted files `scan_cron_files` had already sorted. | Fixed: removed. |
| 23 | `src/machine.rs:312`, `:445` | `flatten` and `parse_machine` were `pub` for one graph unit test. | Fixed: private; the test loads through `load_machine_text`. |
| 24 | `src/interpreter/decide.rs:194` (`wait_id`), `src/commands/init.rs:192` (`router_yml`, `router_ask_sh`) | One-line helpers with one non-test caller. | Kept: they name a reference concept (the wait id, the two router templates) and the init tests use them. |
| 25 | `src/commands/process.rs:256` (`context`), `src/commands/check.rs:61` (`Project`) | Shared project loading and the `Context` constructor live in command modules. | Kept: moving them changes no behaviour and no line count; `process` and `check` are where both start. |
| 26 | `src/error.rs:25` | `DecreeError::Yaml` exists only for `parse_frontmatter`. | Kept: `decree status` still uses it to print a run's frontmatter. |

### Names and shapes from 0.4 or earlier designs

| # | Where | Finding | Status |
| --- | --- | --- | --- |
| 27 | `src/commands/init.rs:28`, `src/cli.rs:122` | "0.4.2's detection order". | Fixed: "the order `init` looks for them on `PATH`". |
| 28 | `src/commands/init.rs:10` | "routine templates". | Fixed: "the develop machines' scripts". |
| 29 | `src/commands/init.rs:112`, `src/runtime.rs:632`, `:634`, `:870`, `:897` | Comments narrating 0.4.2 history ("reused from 0.4.2", "moved here unchanged from 0.4.2"). | Fixed: reworded to what the code does, citing the decision log where one exists. |
| 30 | `src/commands/tail.rs:207` | `max_log_size`, a 0.4 configuration key. | Fixed: "its 2 MiB cap". |
| 31 | `src/runtime.rs:896` | The log truncation comment said "0 disables truncation", which is false. | Fixed: comment corrected. |
| 32 | `src/commands/init.rs:256` | `#[cfg(unix)]` in `write_script`; decree is Unix-only. | Fixed: removed. |
| 33 | `src/machine.rs:408`, `:409` | V19 messages for `router:` and `default:` on a state, keys of the router-state draft that decision invokes replaced. | Kept: `tests/check_test.rs` and `tests/validation_test.rs` (V19) check the `router` message; dropping them changes `decree check` output. |
| 34 | `src/runtime.rs:186`, `:168`, `:171` | `is_reserved_event` (a machine rule) and `MESSAGE_FILE`, `RECEIVED_DIR` (run-folder names) lived in the executor, so `machine.rs` and `message.rs` imported from `runtime`. | Fixed: `machine::is_reserved_event`, `layout::{MESSAGE_FILE, RECEIVED_DIR}`. |
| 35 | `src/interpreter.rs:112` | `MAX_DEPTH` lived in the interpreter but limits `decree emit` too. | Fixed: `message::MAX_DEPTH`. |

### Visibility and module size

| # | Where | Finding | Status |
| --- | --- | --- | --- |
| 36 | `src/interpreter.rs` (1728 lines) | The step loop, decisions, child runs, recovery and status, and event reading in one file. | Fixed: `interpreter.rs` (step loop), `interpreter/{decide,child,recover}.rs`, `events.rs`; tests in `interpreter/tests.rs`. |
| 37 | `src/machine.rs` (1684 lines) | Parsing and the arena, plus V1–V21. | Fixed: `machine/validate.rs`; tests in `machine/tests.rs`. |
| 38 | `src/runtime.rs` (929 lines) | Resolution, the event log and the executor. | Fixed: `runtime/resolve.rs`, `events.rs`. |
| 39 | 29 items: `commands/check.rs:43` `check`, `commands/graph.rs:19` `GRAPH_DIR`, `:70` `render`, `cond.rs:74` `Shape`, `:149`, `:161`, `:185`, `:247`, `:364`, `error.rs:7`, `:8`, `:10`, `:56`, `:81`, `graph.rs:55`, `:235`, `interpreter.rs:244` `resume`, `machine.rs:61` `State` and the other parse types, `:167` `input`, `:690` `final_events`, `runtime.rs:31` `KILL_GRACE`, `:119` `MAX_LOG_SIZE`, `:207` `printed_event`, `:380` `reserve_log`, `:675` `truncate_log_if_needed`, `runtime/resolve.rs:54` `search_dirs` (lines after the split) | `pub` items used only in their own module. | Fixed: private. Each was made private alone and `cargo check --all-targets` stayed clean. |

### Error handling and panics

| # | Where | Finding | Status |
| --- | --- | --- | --- |
| 40 | `src/runtime.rs:113` | **Bug**: a script with no extension named like its machine (`scripts/verify` for machine `verify`) failed V12 with `cannot read scripts/verify: Not a directory`; only `NotFound` counted as "no matches". | Fixed (the one behaviour change, authorised): a per-machine path that is not a directory holds no matches. New case in `tests/validation_test.rs`: "a script with no extension named like its machine". |
| 41 | `src/interpreter.rs:1100` | `claimed_at` indexed events with `e["type"]`, `e["source"]`, `e["ts"]`, which panics on a hand-edited `events.jsonl` missing a key. | Fixed: `claim_event` and `text`. |
| 42 | `src/commands/process.rs:429` | `project.machines[&name]` panics if the machine did not load. | Fixed: an error; unreachable today, since `Pipeline::new` refuses a project with a machine that does not load. |
| 43 | `src/commands/process.rs:157`, `:182`, `:295`, `:478`, `:560`, `src/commands/retry.rs:31` | `read_events(..)?` turned into `io error: ...` without the file. | Fixed: `Context::events` names `runs/<id>/events.jsonl`. |
| 44 | `src/commands/init.rs:175` | `expect("every AiBackend has an AI_BACKENDS entry")`. | Fixed: one constant per backend, matched directly. |
| 45 | `src/reply.rs:106` | `expect("a waiting run has events")`. | Fixed: a `let ... else` that rejects the reply. |
| 46 | `src/interpreter.rs:573` | `unreachable!` after `run_invoke` returned `None` for a script state. | Fixed: `run_invoke` takes the script name and has no `None` case; its two unit tests of the `None` path are gone. |
| 47 | `src/lib.rs:67` | `unreachable!()` for `init` and `help` in the project branch of `dispatch`. | Fixed: one `match`, each project command calls `require_project_root`. |
| 48 | `src/cond.rs:313`, `:318`, `:325` | Three `unreachable!` in `Condition::eval`. | Fixed: `eval` matches `(subject, test)`; the impossible shapes return `NoOperator`. |
| 49 | `rg -n '\.unwrap\(\)\|\.expect\(' src --glob '!**/tests.rs'` | After the fixes above, every hit is inside `#[cfg(test)]` code (the test modules of `cond.rs`, `cron.rs`, `error.rs`, `graph.rs`, `message.rs`, `runtime.rs`, `runtime/resolve.rs`, `commands/{daemon,init,retry,status,tail}.rs`). | Fixed: none outside tests. |
| 50 | `src/runtime.rs:423` | Spawning a script can fail with ETXTBSY if another thread of the same process holds a write handle to it; only test binaries write and run scripts in parallel. | Kept: a retry on ETXTBSY is optional hardening; `tests/README.md` describes the rule tests follow instead. |

### Tests

| # | Where | Finding | Status |
| --- | --- | --- | --- |
| 51 | `src/commands/init.rs:400`, `:462`, `:474`, `:571` | Unit tests that repeat black-box tests: the layout (`integration_test.rs` `test_init_creates_directory_structure`, `test_init_processed_md_is_empty`), `.gitignore` (`test_init_gitignore_content`), the claude router against the mock (`mock_templates_test.rs`), and the copilot router (`test_init_ai_opencode_and_copilot_write_their_routers`, where `decree check` also checks the script is executable). | Fixed: removed. |
| 52 | `src/commands/init.rs` (other skill and backend tests) | The skill tests check every template file, the repository's installed copies and the mock examples; the backend tests check detection order and placeholders for all three backends. | Kept: they cover more than `integration_test.rs`. |
| 53 | `src/machine/tests.rs:484` and the other V1–V21 unit tests | Validation unit tests next to the `validation_test.rs` table. | Kept: they test `validate`, a pure function, with many cases per rule; the table has one passing and one failing case per rule. |
| 54 | `tests/integration_test.rs:425`, `tests/cli_test.rs:684` | `integration_test.rs` and `cli_test.rs` both cover `init` and the CLI. | Kept: no test repeats another (bare `decree` and the project commands are different checks); merging the files is optional. |
| 55 | `src/commands/init.rs:400`, `:439`, `src/machine.rs:1728`, `:2080`, `src/runtime.rs:1468`, `:1600`, `src/interpreter.rs:3665`, `:4121`, `tests/check_test.rs:54`, `:161` | Test names citing spec sections (`section_3`, `section5`, `0_4_2_order`). | Fixed: renamed to what they check. |
| 56 | `tests/check_test.rs:159`, `:218`, `tests/interrupt_test.rs:146` | Comments citing migrations 47 and 48 and "section 8". | Fixed: removed or replaced with the reference file. |
| 57 | `src/runtime.rs:1691` | `stop_escalates_to_sigkill_after_the_grace_period` waited the real 10 s grace period; the `src` unit tests took 11.8 s. | Fixed: `stop_group` takes the grace period (`KILL_GRACE` in production); the test uses 300 ms, and a separate test pins `KILL_GRACE` at 10 s. The unit tests now take 1.8 s. The next slowest take about 1 s each because `timeout_s` is whole seconds. |

### Outside `src/`

| # | Where | Finding | Status |
| --- | --- | --- | --- |
| 58 | `mock/README.md:3` | "Nothing here runs yet ... once 0.5 ships", stale after 0.5. | Fixed: the mock is described as a snapshot. |
| 59 | `README.md:273` (Docker) | The image has no Rust toolchain, so `rust_develop` (whose scripts run `cargo`) cannot run in it. | Fixed: noted in the Docker section. |
| 60 | `src/commands/process.rs:460` | `process` prints a waiting run's question from the state's `description`, not the `choose: person` invoke's `question`. | Kept: `docs/reference/messages.md` (Replies) says "its question (the state's `description`)" and `docs/reference/cli.md` gives the format `<the state's description, else its id>`. |

## Summary

60 findings: 49 fixed, 11 kept. Findings 15 (cron files used 0.4's frontmatter parser) and 16 (`tail` read only 4-digit log numbers) changed behaviour and were fixed after the review, by decision.
