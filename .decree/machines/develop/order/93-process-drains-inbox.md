---
machine: rust_develop
---
# 93: `decree process` keeps draining the inbox after a failed inbox run

## Overview

Found at work: a migration emitted several ComfyUI messages; one render failed, and `decree process` exited, leaving the rest in `inbox/` until a second `decree process`. Inbox messages are independent of each other, and `decree daemon` already reports a failed run and goes on. The user decided: `process` does the same for inbox runs. Migrations keep stopping on error, because they are ordered.

## Requirements

1. **`process`:** a run claimed from `inbox/` (including a reply's rejection run and a `pending` run continued from an earlier reply) that ends in `failed` is reported, with its `decree process --retry <id>` command, and `process` goes on with the next inbox file. After the queue is empty, `process` prints every run that failed during this pass, and exits 1 if there was at least one, else as today.
2. **Migrations are unchanged:** a migration whose run fails stops `process` at once, with today's message and exit 1, and blocks later migrations. A migration also does not start while its predecessor's emitted inbox messages are still queued (today's rule); a failed emitted message does not block the next migration, since it has left the inbox.
3. **Retry is unchanged:** `process --retry <id>` continues that run first, then the pass continues.
4. **Docs:** `docs/reference/cli.md` (process row and exit codes), `docs/reference/messages.md` (Lifecycle and Migrations: say plainly that a failed inbox run does not stop `process`, a failed migration does), `src/templates/help.txt`, the skill's `reference/messages.md`, and a CHANGELOG "Changed" entry.
5. **Tests:** three inbox messages, the middle one fails: all three run, the output lists the failed one with its retry command, exit 1. A failing migration still stops `process` before the next migration. A passing pass still exits 0.

- Only this migration's scope.
- Never edit `.decree/migrations/` or `.decree/runs/`, not even by a search and replace across the repository: exclude both from every bulk edit.
- If the reference docs and this migration disagree in a way you cannot settle, write the question to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- No test calls a real model or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass, and `decree check` passes in every example directory.

## Acceptance Criteria

- **Given** three inbox messages where the second one's script fails
  **When** `decree process` runs
  **Then** all three runs exist, the first and third finished in `done`, the output names the second with `decree process --retry <id>`, and the exit code is 1

- **Given** two migrations where the first fails
  **When** `decree process` runs
  **Then** it stops after the first, exit 1, and the second has no run
