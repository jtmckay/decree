---
machine: develop
---
# 107: `decree stop` holds the queue through `.decree/attention/`

## Overview

Ctrl-C (SIGINT) or SIGTERM is the only way to stop `process` or `daemon`, and it interrupts the run that is going, which then needs `decree process --retry`. A beta user wanted "finish this run, then stop". The user wants one place to check for everything that needs them, `.decree/attention/` (migration 106), and no other control files in `.decree/`. So `decree stop` writes an attention file that holds the queue, and deleting it resumes. Prior art for a graceful stop: `nginx -s quit` (finish the requests in flight, then exit) and Sidekiq's quiet signal (take no new jobs); for a marker file that blocks until removed, git's `MERGE_HEAD`.

## Requirements

1. **`decree stop [-m <note>]`** writes `.decree/attention/manual-stop.md` with frontmatter `kind: manual-stop`, `since` (RFC 3339 UTC) and `holds: true`, and a body: `decree stop` was run (with the note, if given); running runs finish their current step loop and no new run starts; delete this file to resume. It prints `stopping after the current run; delete .decree/attention/manual-stop.md to resume`. Exit 0; exit 1 outside a project. If the file exists already, it is left as it is (the first `since` stays) and the command says so, exit 0. `--format json` prints `{"attention": "<path>"}`, with `schema/v1/cli/stop.schema.json`.
2. **The attention schema** (`schema/v1/attention.schema.json`) gains this second shape: `kind: manual-stop` with `since` and `holds`, no run fields. A run's file gets `kind: run`. `decree status --format json` lists both under `attention`.
3. **It holds like any holding file** (106): `process` and `daemon` check before a run starts or continues; the run in progress is not interrupted, it reaches a final state or pauses `waiting`. Then `process` prints the file and exits 1, as for any hold, and a new `process` started while the file exists starts nothing. `daemon` prints it once and goes on polling; when the file is deleted it continues, with no restart. `decree process --retry <id>` does not remove `manual-stop.md`: the retried run waits as `pending` until the file is deleted.
4. **Nothing else.** decree writes no other file outside `attention/` for this: no `.decree/stop`, no pid list, no signal.
5. **Docs**: a `stop` row in `docs/reference/cli.md` and a "Stopping" paragraph under "`process` or `daemon`" (Ctrl-C interrupts the run; `decree stop` lets it finish and holds the queue until its attention file is deleted); `help.txt`; the skill's Commands table; CHANGELOG under Added; decision D68 in `docs/decisions.md` (a graceful stop as an attention file: one place for a person to look, deleting it resumes, a daemon needs no restart).
6. **The skill** (`src/templates/skills/decree/SKILL.md`, then `decree skill` refreshes `.claude/skills/decree/`): a `decree stop` row in the Commands table; the attention rule (migration 106) adds: to pause the queue, `decree stop`, and delete `.decree/attention/manual-stop.md` to resume; Ctrl-C interrupts the run instead.

- Only this migration's scope.
- Never edit `.decree/migrations/`, `.decree/runs/` or `.decree/processed.md`, not even by a search and replace across the repository: exclude them from every bulk edit.
- If the reference docs and this migration disagree in a way you cannot settle, write the question to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- No test calls a real model or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass, and `decree check` passes in every example directory.

## Acceptance Criteria

- **Given** two inbox messages and a `process` running the first
  **When** `decree stop` runs before the first finishes (in the test, a script of the first run calls `"${DECREE}" stop`)
  **Then** the first run ends in `done`, the second is still in `inbox/`, `process` prints the `manual-stop.md` hold and exits 1, and the file validates against `attention.schema.json`

- **Given** `.decree/attention/manual-stop.md` exists
  **When** a new `decree process` runs
  **Then** it starts nothing and exits 1; after the file is deleted, the next `process` runs the queue

- **Given** a `daemon` held by `manual-stop.md`
  **When** the file is deleted
  **Then** the daemon runs the next queued message without being restarted

- **Given** the repository
  **When** `rg -n --hidden '\.decree/stop\b' --glob '!.decree/migrations/**' --glob '!.git/**'` runs
  **Then** it matches nothing
