---
machine: rust_develop
---
# 85: `decree process --retry` replaces `decree retry`

## Overview

The user does not want a separate `decree retry` command. The behaviour stays the same:

- decree never continues a failed or interrupted run by itself;
- a failed or interrupted migration blocks later migrations;
- continuing a run keeps its run folder and history.

What changes is how you ask for it. The common case is "continue the migration that is blocking the queue", and that is now a flag on the command you were going to run anyway. Every message that tells you to continue a run prints the exact command.

## Requirements

Read `docs/reference/cli.md` (`process`, `retry`, `status`, `daemon`), `docs/reference/messages.md` (Lifecycle, Run status, Migrations), `docs/reference/runs.md` (Step loop), `src/commands/retry.rs`, `src/commands/process.rs` and `src/commands/status.rs` first.

1. **`decree process --retry [<id>] [--state <s>]`**
   - **Without an id:** the run to continue is the migration that blocks the queue: the earliest pending migration whose run is `failed` or `interrupted` (`docs/reference/messages.md`, Migrations, rule 4). If there is none, exit 1 with `nothing to retry: no migration is failed or interrupted`.
   - **With an id:** that run, whatever started it (a migration, an inbox message, cron or a child run).
   - **Then:** do exactly what `decree retry` did: append the `transition` with `source: "retry"` to the run's `events.jsonl`, mirror `state`, with the same default state and the same `--state` rules and errors. After that, continue with the normal `process` pipeline in the same invocation, so the run and everything queued after it are processed.
   - **Exit codes:** as `process` itself; and 1 for a run that is `active`, `pending` or `waiting` (waiting runs take a reply), for an unknown id, or for a `--state` that is not an atomic state.
   - **Other flags:** `--retry` may not be combined with `--dry-run` (usage error, exit 2). `--state` needs `--retry` (usage error, exit 2).
2. **Remove `decree retry`.** Delete:
   - `src/commands/retry.rs`, moving its logic to where `process` can call it;
   - the subcommand;
   - `src/templates/schema/v1/cli/retry.schema.json` and its entry in `decree schema`;
   - every test of the old command. Rewrite each one against `decree process --retry` with the same expectations; do not drop a case.
   
   `decree retry` then fails as an unknown command (exit 2), as clap reports it. Delete files with `git rm` or `rm`. If deleting is not allowed to you, list the paths in `progress.md`, leave the files unreferenced, and say so at the end of your reply; a person will delete them.
3. **Every message that suggests continuing a run** names the new command, exactly:
   - `process` and `daemon` stopping on a blocked migration: `Fix the cause, then run \`decree process --retry\`.`;
   - `process` stopping on a failed inbox run: `decree process --retry <id>`;
   - `decree status` for an interrupted or failed run: `continue with \`decree process --retry <id>\``, or with no id when the run is the blocking migration.
   
   Find them all: `rg -n 'decree retry' src`.
4. **`events.jsonl` is unchanged:** `source: "retry"` keeps its name and meaning. It records that a person continued the run.
5. **Docs:** replace `decree retry` everywhere it is current:
   - `docs/reference/` (the `cli.md` table: drop the `retry` row, add the flag to the `process` row), `README.md`, `help.txt`, the skill, `examples/*/README.md`, `tests/README.md`;
   - the schema descriptions and the script templates' comments (`rust_develop/implement.sh`, `git_baseline.sh`).
   
   `docs/decisions.md` and `docs/code-review.md` keep their history. Add a `CHANGELOG.md` "Changed" entry and a decision entry for the new command shape, saying why: one command to remember, and the failure message prints the exact command.
6. **Tests**, through the binary:
   - with a migration failed and a later one pending, `decree process` exits 1 and prints `decree process --retry`;
   - `decree process --retry` continues the failed migration in the same run folder, so its earlier events are kept, then runs the later migration;
   - `--retry <id>` on a failed inbox run continues it;
   - `--retry --state <s>` resumes at `<s>`;
   - `--retry` with nothing to retry exits 1 with the message;
   - `--retry` on a waiting run exits 1;
   - `--retry --dry-run` and `--state` without `--retry` exit 2;
   - `decree retry` exits 2.

- Only this migration's scope.
- If the reference docs and the code disagree, or a case is not covered here, do not guess: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- Tests build their own `.decree/` in a temp directory and never touch this repository's `.decree/`. No test calls a real LLM or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass.

## Acceptance Criteria

- **Given** a failed migration blocking the queue
  **When** `decree process` runs
  **Then** it exits 1 and prints `decree process --retry`; and `decree process --retry` continues that run in its own folder and then processes the rest

- **Given** the repository
  **When** `rg -n 'decree retry' src tests docs/reference README.md examples` runs
  **Then** nothing matches

- **Given** `decree retry`
  **When** it runs
  **Then** it exits 2 as an unknown command
