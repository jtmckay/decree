---
machine: develop
---
# A03: Migration runs get time ids like every other run

## Overview

A migration's run id is its file stem (`runs/01-rate-limit-upload/`), while every other run's id is the time it was made. So `runs/` and `decree status` sort migration runs apart from the rest, and a migration's numbering (`99`, `100`, `A01`) leaks into run order. The user wants one kind of run id. A migration's name still matters for finding its run, so names keep working wherever a person types them.

## Outcome

- Every run, a migration's included, has a time id. A migration's run records the file it came from as `migration: <file>` in its `message.md`.
- `migration:` is decree's alone, like `id`: a message in `inbox/` that carries it (a copy of a migration's run) loses it, and its run is not a run of that migration.
- What a migration's run decides is unchanged: a migration runs once (`processed.md`), in order, and a failed, interrupted or waiting one blocks later migrations; a migration that has a run is not started again.
- Wherever a person names a run (`decree process --retry`, `decree daemon --retry`, `decree status <id>`, `decree tail <id>`), a migration's file name or stem works too, and means that migration's newest run.
- Everything that shows a migration's run shows its file name with it: status lines, `process` notices, the attention file, the commit title the built-in `commit` writes.
- The examples' recorded runs and the test fixtures use time ids for their migration runs.

## Constraints

- Run folders that already exist are never renamed (they are the record): this repository keeps its old migration runs as they are, and decree still recognises them as the runs of their migrations.
- `decree prune` still never deletes a failed migration's run.
- Only this migration's scope.
- Never edit `.decree/migrations/`, `.decree/runs/` or `.decree/processed.md`, not even by a search and replace across the repository: exclude them from every bulk edit.
- If the reference docs and this migration disagree in a way you cannot settle, write the question to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- No test calls a real model or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass, and `decree check` passes in every example directory.
- CHANGELOG, the reference docs, the skill (and this repository's copy, as `decree skill` writes it) and a decision in `docs/decisions.md` say what changed and why.

## Acceptance Criteria

- **Given** a pending migration `01-a.md`
  **When** `process` runs it
  **Then** its run folder is named by a time id, its `message.md` has `migration: 01-a.md`, and `processed.md` lists `01-a.md`

- **Given** `01-a.md` failed
  **When** `decree process --retry 01-a` runs (and, separately, `--retry 01-a.md`)
  **Then** that run continues, and `decree status 01-a` shows it

- **Given** an emitted message, then a migration, then another emitted message
  **When** all three have run
  **Then** `decree status --all` lists their runs in that order

- **Given** a project with a migration run folder named by its stem from before this change
  **When** `process` and `status` run
  **Then** the migration is still recognised as run, and is not started again
