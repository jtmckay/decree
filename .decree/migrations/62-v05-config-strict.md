---
routine: rust-develop
---
# 62: v0.5 cleanup: strict config.yml

## Overview

`config.yml` still behaves like 0.4 in three ways the spec rules out: unknown keys are ignored, the 0.4 key names are still read, and `default_machine` can never be unset. 0.5.0 is a breaking release with no compatibility shims for 0.4 config keys (spec section 1, rule 4), so make the code match the spec.

## Requirements

Read `docs/0.5-spec.md` sections 1 (rule 4), 3 (`config.yml`), 4 (Frontmatter keys, Validation M1–M3) and 11 (M5.4) first, then `src/config.rs` and every use of its fields.

1. **Unknown keys are an error.** Add `#[serde(deny_unknown_fields)]` to the config type. A config that fails to parse makes every command that reads it exit 1 with `config.yml: <serde's message>`. When the unknown key is one 0.4 used (`routines`, `shared_routines`, `hooks`, `commands`, `default_routine`, `routine_source`, `max_retries`), add: `this is a 0.4 config; run scripts/migrate-0.4-to-0.5.sh`.
2. **No 0.4 aliases.** Drop `alias = "max_retries"`, and the aliases that read `default_routine` and `routine_source`. Rename the fields to the section 3 names (`default_machine`, `shared_source`) and update every use and comment, including the "read until M4.1" comments in `src/config.rs`, `src/commands/check.rs` and `src/commands/graph.rs`.
3. **`default_machine` is optional.** No default value: when it is unset, a message without `machine:` (or `routine:`) fails validation (M1 for migrations, M2 for inbox files, M3 for cron files) with the message the spec gives, and `decree process --dry-run` shows it as invalid. `decree init` still writes `default_machine: develop`.
4. Make sure `scripts/migrate-0.4-to-0.5.sh` still produces a config that 0.5 accepts, and that `mock/`, every project in `examples/` and what `decree init` writes still pass `decree check`.

- Only this migration's scope; migrations 63–67 cover the rest of the cleanup.
- If the code does something the spec does not cover, or the spec is ambiguous here, do not guess: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes. The routine fails the run when `STOP` exists.
- Tests build their own `.decree/` in a temp directory and never touch this repository's `.decree/`. No test calls a real LLM or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass. The routine checks this itself after you finish.

## Files to Modify

- src/config.rs
- src/commands/ (field renames, error output)
- src/message.rs (validation without a default machine)
- tests/

## Acceptance Criteria

- **Given** a fresh `decree init` with `bogus: 1` appended to `config.yml`
  **When** `decree check` runs
  **Then** it exits 1 and names `bogus`

- **Given** a config with `routines: {}` or `default_routine: develop`
  **When** `decree check` runs
  **Then** it exits 1, and the message says to run `scripts/migrate-0.4-to-0.5.sh`

- **Given** a config without `default_machine` and an inbox message with no `machine:`
  **When** `decree check` and `decree process --dry-run` run
  **Then** both report that message as invalid (M2), and nothing runs

- **Given** `mock/`, every project in `examples/`, a fresh `decree init`, and a 0.4 fixture after `scripts/migrate-0.4-to-0.5.sh` with its machines added
  **When** `decree check` runs
  **Then** it exits 0 for each

- **Given** the repository
  **When** `rg -n -w 'default_routine|routine_source|max_retries' src tests` runs
  **Then** it finds only the 0.4 key names listed in the error message for item 1, and their tests
