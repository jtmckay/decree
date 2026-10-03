---
routine: rust-develop
---
# 57: v0.5 M5.4 0.4 to 0.5 layout script

## Overview

Upgrade existing projects once, outside the binary. Part of the 0.5.0 rewrite specified in `docs/0.5-spec.md` (ticket M5.4).

## Requirements

Read spec sections 1 and 3, 4, 11 (M5.4) first.

Write `scripts/migrate-0.4-to-0.5.sh` as section 11 M5.4 describes. Leave `migrations/` and `processed.md` untouched. Do not run it on this repository's `.decree/`.

- Only this migration's scope; the other v0.5 migrations cover the rest of spec section 11.
- If the code does something the spec does not cover, or the spec is ambiguous here, do not guess: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes. The routine fails the run when `STOP` exists.
- Tests build their own `.decree/` in a temp directory and never touch this repository's `.decree/`. No test calls a real LLM or the network. No new dependencies unless the spec names them.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass. The routine checks this itself after you finish.

## Files to Modify

- scripts/migrate-0.4-to-0.5.sh — new
- tests/fixtures/legacy-0.4/ — new

## Acceptance Criteria

- **Given** a 0.4.2 project fixture in a temp directory whose pending migration, inbox message and cron file name `develop`
  **When** the script runs
  **Then** it exits 1 and lists `develop` with those three files

- **Given** the same fixture after the script, with `machines/develop.yml` and its scripts added
  **When** `decree check` and `decree status` run
  **Then** both succeed

- **Given** a pending migration with `routine: rust-develop`
  **When** the script runs
  **Then** it lists `rust-develop` as not a valid machine name

- **Given** the same fixture
  **When** the script runs
  **Then** `migrations/` and `processed.md` are byte-identical to before, 0.4 run folders are in `.decree/legacy-0.4/runs/`, and `.decree/.gitignore` is `inbox/` and `runs/`
