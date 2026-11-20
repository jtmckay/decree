---
machine: develop
---
# 109: Each run records the decree version that started and retried it

## Overview

A beta user upgraded decree partway through a run and retried it, and nothing recorded which version ran which step. Git history already records the machines and scripts (one commit per run, D63); traces carry `service.version`, but `events.jsonl`, the run's own record, does not. The user chose to record the version only: no hashes of machines or scripts, no warnings, no `version:` field.

## Requirements

1. The claim event (`source: "claim"` or `invalid_message`) and every `retry` event get `decree_version`: the running binary's `CARGO_PKG_VERSION` (for example `0.5.0-beta.11`).
2. `schema/v1/events.schema.json`: the optional string `decree_version` on `transition` events, documented as present on the claim and `retry` only; an additive change (`schema/CHANGELOG.md`). `docs/reference/runs.md` event table gets the row.
3. `decree status <id>` shows the version on the claim and `retry` rows.
4. Recorded fixtures without the field stay valid. CHANGELOG under Added.

- Only this migration's scope.
- Never edit `.decree/migrations/`, `.decree/runs/` or `.decree/processed.md`, not even by a search and replace across the repository: exclude them from every bulk edit.
- If the reference docs and this migration disagree in a way you cannot settle, write the question to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- No test calls a real model or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass, and `decree check` passes in every example directory.

## Acceptance Criteria

- **Given** a new run
  **When** it starts
  **Then** its first event has `decree_version` equal to `env!("CARGO_PKG_VERSION")`

- **Given** a failed run
  **When** `decree process --retry <id>` runs
  **Then** the `retry` event has `decree_version`, and `decree status <id>` shows it

- **Given** the recorded fixtures under `tests/fixtures/`
  **When** they are validated against `events.schema.json`
  **Then** they pass
