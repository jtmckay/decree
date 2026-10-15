---
machine: develop
---
# 110: `decree check` warns about a migration inserted before ones that ran

## Overview

A beta user added `B03b`, which sorts between migrations that had already run. That is allowed, and was intended, but nothing made it visible. Migrations run in filename order and each runs once, so an inserted migration runs next, after work that sorts later. The user chose a warning, not an error: inserting is sometimes deliberate.

## Requirements

1. `decree check` warns, for each pending migration (not in `processed.md`) whose filename sorts before the last migration listed in `processed.md` by the same order `process` uses: ``migration <file> sorts before <last>, which already ran; it runs next``. Exit code unchanged (0 when there is nothing else).
2. The warning has its own rule id, following the existing warnings (store warnings) in text, `--format json` and `--format sarif` (level `warning`).
3. `docs/reference/messages.md` (Migrations) and the check rules table get the warning. CHANGELOG under Added.

- Only this migration's scope.
- Never edit `.decree/migrations/`, `.decree/runs/` or `.decree/processed.md`, not even by a search and replace across the repository: exclude them from every bulk edit.
- If the reference docs and this migration disagree in a way you cannot settle, write the question to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- No test calls a real model or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass, and `decree check` passes in every example directory.

## Acceptance Criteria

- **Given** `processed.md` lists `01-a.md` and `03-c.md`, and `02-b.md` is pending
  **When** `decree check` runs (text, `--format json`, `--format sarif`)
  **Then** each names `02-b.md` and `03-c.md` as a warning, and the exit code is 0

- **Given** `processed.md` lists `01-a.md` and only `02-b.md` is pending
  **When** `decree check` runs
  **Then** there is no such warning
