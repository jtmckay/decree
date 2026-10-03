---
routine: rust-develop
---
# 66: v0.5 cleanup: tests read the mock, validation tests as a table

## Overview

`tests/` has 241 files. Most are fixture directories for the validation rules (a `pass/` and `fail/` project per rule, with 56 identical `work` scripts), copies of mock machines in `tests/fixtures/machines/` and `tests/fixtures/graph/`, and `blackbox_test/test_decree.sh` overlaps the Rust tests. Make the tests smaller and easier to read without losing a single case.

## Requirements

Read `docs/reference/machines.md` (Validation), `docs/reference/messages.md`, `docs/reference/cli.md` and `tests/` first.

1. Tests that use a mock machine or graph read it from `mock/` directly. Delete the copies in `tests/fixtures/machines/` and `tests/fixtures/graph/` that equal a mock file.
2. Replace `tests/fixtures/check/` with one table-driven test file, `tests/validation_test.rs`. Each case is a rule id, a short name, the machine YAML (and any message, inbox or cron file) inline as a string, and the exact expected `decree check` output (or "passes"). A helper writes the case into a temp project and creates a stub executable for every script name the machine references, unless the case says a script is missing or not executable. Keep every existing pass and fail case, with the same expected messages, and add a case for any rule V1–V21 or M1–M3 that has fewer than one pass and one fail.
3. Move every case in `blackbox_test/test_decree.sh` that `tests/` does not already cover into a Rust test that runs the built binary, then delete `blackbox_test/`. Say which cases were already covered.
4. Write `tests/README.md`: one line per test file saying what it covers, and how to add a validation case.

- Only this migration's scope; migrations 67–68 cover the rest of the cleanup.
- Change no behaviour of the binary, and no expected output. If an existing test's expectation looks wrong, do not change it: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes. The routine fails the run when `STOP` exists.
- Tests build their own `.decree/` in a temp directory and never touch this repository's `.decree/`. No test calls a real LLM or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass. The routine checks this itself after you finish.

## Files to Modify

- tests/validation_test.rs — new
- tests/fixtures/check/, tests/fixtures/machines/ (mock copies), tests/fixtures/graph/ (mock copies), blackbox_test/ — delete
- tests/*.rs — read mock/ directly
- tests/README.md — new

## Acceptance Criteria

- **Given** the list of test cases before this migration (print it: `cargo test -- --list`, plus each blackbox case)
  **When** it is compared with the list after
  **Then** every case still exists (renamed is fine) or is named as covered by another test

- **Given** each rule V1–V21 and M1–M3
  **When** `tests/validation_test.rs` is read
  **Then** it has at least one passing and one failing case for that rule

- **Given** the repository
  **When** `git ls-files tests blackbox_test | wc -l` runs
  **Then** the count is far below 241 (print before and after)
