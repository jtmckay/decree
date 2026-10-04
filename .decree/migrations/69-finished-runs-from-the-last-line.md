---
machine: rust_develop
---
# 69: Finished runs cost one small read

## Overview

Every `process` loop and every `daemon` pass reads and parses the whole `events.jsonl` of every run in `runs/`, finished ones included: to recover crashed runs, to deliver `timeout_s` deadlines, to find `pending` runs, and to print waiting runs. Measured on copies of a finished mock run (27 events), an idle `decree process` takes 0.12 s with 1,000 runs, 0.66 s with 5,000 and 2.6 s with 20,000, and `decree status` 0.03 s, 0.16 s and 0.6 s. Within `process`, the per-loop scans repeat for every message, so at 20,000 runs each message costs about 1.3 s extra, and a daemon at its 2 s default interval spends about half its time re-reading finished runs.

A finished run's last line is its `run_finished` event, and nothing changes a finished run except `decree retry`, which appends a `transition` after it. So a run whose last line is `run_finished` is finished, without reading the rest.

## Requirements

Read `docs/reference/messages.md` (Lifecycle, Run status, Run lock), `docs/reference/runs.md` (`events.jsonl`), `tests/README.md`, then `src/events.rs`, `src/interpreter/recover.rs`, `src/reply.rs`, `src/commands/process.rs` and `src/commands/status.rs`.

1. **Measure first.** Build a release binary and a temp project: `mock/.decree` with an empty inbox, plus N copies of `mock/.decree/runs/01-rate-limit-upload/` under new run ids, for N = 1,000, 5,000 and 20,000. Time an idle `decree process` and `decree status` for each. Keep the script in the run directory, not in the repository, and print the table.
2. **Read the last line.** Add a function in `src/events.rs` that returns a run's last event by reading backwards from the end of `events.jsonl` (seek, then read blocks until a full line is found), without reading the whole file. Every scan over all runs (crash recovery, timeouts, pending, the waiting list at the end of `process`, `decree status` without an id, `decree tail` choosing a run) first reads the last event. If it is `run_finished`, the run is finished: skip it, or count it as finished, without the full parse or the lock check. Otherwise do exactly what the code does today.
3. Change no behaviour. The status of every run, every command's output and every event written stay the same. `decree status <id>` and the interpreter still read the whole log.
4. **Measure again** with the same script and print both tables. Idle `decree process` with 20,000 finished runs should be at least 10 times faster.
5. Tests:
   - unit tests of the last-event reader: one line, many lines, a file that does not end in a newline, a last line longer than one read block, an empty or missing file;
   - a black-box test that proves finished runs are not parsed: a finished run whose `events.jsonl` has a line in the middle that is not JSON is still counted as finished by `decree status` and does not stop `decree process` (today both fail on it, so the test fails before the change);
   - a run whose last line is a `transition` written by `decree retry` after `run_finished` is still `pending` and continues.
6. Update `docs/reference/messages.md` (Run status) to say that a run whose last event is `run_finished` is finished, and that decree reads only that line for it.

- Only this migration's scope; migration 70 adds `decree prune`.
- If the reference docs and the code disagree, or a test looks wrong, do not guess: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- Tests build their own `.decree/` in a temp directory and never touch this repository's `.decree/`. No test calls a real LLM or the network. No new dependencies. No timing assertions in `cargo test`.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass.

## Files to Modify

- src/events.rs — last-event reader
- src/interpreter/recover.rs, src/reply.rs, src/commands/process.rs, src/commands/status.rs, src/commands/tail.rs — the fast path
- docs/reference/messages.md
- tests/

## Acceptance Criteria

- **Given** 1,000, 5,000 and 20,000 finished runs
  **When** an idle `decree process` and `decree status` run before and after
  **Then** both tables are printed, and idle `process` at 20,000 runs is at least 10 times faster

- **Given** a finished run with a corrupt line in the middle of `events.jsonl`
  **When** `decree status` and `decree process` run
  **Then** it is counted as finished and nothing fails (the test fails before the change)

- **Given** a finished run that `decree retry` made `pending`
  **When** `decree process` runs
  **Then** it continues as before

- **Given** every other test in `tests/`
  **When** `cargo test` runs
  **Then** they pass unchanged
