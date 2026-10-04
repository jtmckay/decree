---
machine: rust_develop
---
# 70: `decree prune`

## Overview

Run folders are never removed, so `runs/` grows by every run's events, logs and replies (about 64 KB for a mock migration run). Decided: decree deletes finished runs, but only when a person runs `decree prune`; nothing deletes them automatically. Retention is the job of the log store: a project that ships `events.jsonl` and the script logs to Loki ([observability.md](../../docs/reference/observability.md)) keeps its history there, so the local folder is a working copy. Archiving is not offered; it would only move the growth. Prior art: `docker system prune --filter until=<duration>`, `git gc --prune=<date>`, and AWS Step Functions, which keeps execution history for a fixed period and then deletes it.

## Requirements

Read `docs/reference/messages.md` (Lifecycle, Run status, Run lock, Migrations), `docs/reference/runs.md` (Sub-machines), `docs/reference/cli.md`, `docs/reference/observability.md` and `tests/README.md`, then `src/commands/` and `src/interpreter/recover.rs`.

1. `decree prune --older-than <age> [--dry-run]` deletes `runs/<id>/` for every run that is finished and whose `run_finished` event's `ts` is older than `<age>`. `<age>` is a whole number followed by `d`, `h` or `m` (`30d`, `12h`, `90m`); `0m` means every finished run. `--older-than` is required, so a bare `decree prune` never deletes anything.
2. Never pruned, even when old enough:
   - a run that is not finished (`active`, `waiting`, `pending`, `interrupted`);
   - a migration run that ended in `failed`: its file is not in `processed.md`, and deleting its folder would let the next `process` run the migration again from scratch instead of waiting for `decree retry`;
   - a child run (its `message.md` has `parent`) whose parent run still exists and is not finished: the parent may still read its child's results.
3. For each run: take the run lock ([Run lock](../../docs/reference/messages.md#run-lock)); if the lock is held, skip the run. Check the conditions again under the lock, then delete the folder.
4. Output, one line per run, in `id` order: `pruned <id>  <machine>  <final state>  finished <ts>`, or `would prune …` with `--dry-run`. Then a summary: `pruned N run(s), <size> freed` (or `would prune N run(s), <size>`), with the size in KB, MB or GB of the files deleted. Runs skipped by rule 2 are not listed. Exit 0, or 1 on an I/O error (after pruning what it could), or 2 for a bad `<age>`.
5. Docs:
   - `docs/reference/cli.md`: a row for `decree prune`;
   - `docs/reference/messages.md` Lifecycle step 5: the folder is kept until `decree prune` removes it;
   - `docs/reference/observability.md`: ship runs to Loki before pruning them; Loki's retention is the history;
   - `docs/decisions.md`: a new entry in the same Nygard form (context, decision, consequences) for "delete finished runs on request, never automatically", with the prior art above;
   - `src/templates/help.txt` and the decree skill's CLI reference (`src/templates/skills/decree/`), if they list the commands.
6. Tests through the binary, in `tests/`:
   - only finished runs older than `<age>` are deleted, using runs whose `run_finished` `ts` is set in the past;
   - `--dry-run` deletes nothing and prints the same runs;
   - each rule 2 case is kept;
   - a run whose lock is held by a live process is kept;
   - a bad `<age>` (`30`, `d`, `-1d`, `1w`) exits 2 and deletes nothing;
   - after pruning a finished migration's run, `decree process` does not run it again (it is in `processed.md`).

- Only this migration's scope.
- If the reference docs and the code disagree, or a case is not covered above, do not guess: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- Tests build their own `.decree/` in a temp directory and never touch this repository's `.decree/`. No test calls a real LLM or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass.

## Files to Modify

- src/cli.rs, src/commands/prune.rs (new), src/commands/mod.rs
- docs/reference/cli.md, docs/reference/messages.md, docs/reference/observability.md, docs/decisions.md
- src/templates/help.txt, src/templates/skills/decree/
- tests/

## Acceptance Criteria

- **Given** finished runs on both sides of `--older-than`, and one run of each rule 2 case
  **When** `decree prune --older-than 30d` runs
  **Then** exactly the old finished runs are deleted, the output lists them and the size freed, and it exits 0

- **Given** the same project
  **When** `decree prune --older-than 30d --dry-run` runs
  **Then** nothing is deleted and the same runs are listed with `would prune`

- **Given** `decree prune` without `--older-than`, or with a bad age
  **When** it runs
  **Then** it exits 2 and deletes nothing

- **Given** a pruned finished migration run
  **When** `decree process` runs
  **Then** the migration does not run again
