---
machine: develop
---
# 111: `clean_tree` names the run that left changes, and how to set them aside

## Overview

A beta user interrupted `C01` (Ctrl-C), then wanted `B03b` to run first. `clean_tree` refused, because `C01`'s files were still in the tree, and the user had to work out which run they belonged to and move them aside by hand. An interrupted run never reaches `failed`, so `keep` never ran and nothing printed the stash command. The user chose no new command: git already parks work (`git stash`). The fix is in the scripts, where git already is; decree's core does not call git.

## Requirements

1. When `clean_tree.sh` finds changes, it looks for a run that passed `clean_tree` (its run directory has the `clean_tree` mark) and is now `interrupted` or `failed`, using `"${DECREE}" status --all --format json` (never by parsing text). For each one found, it prints the run's id and first heading, `decree process --retry <id>` to continue it on the changes, and `git stash push --include-untracked -m "decree <id>" -- .` to set them aside. With none found, it prints what it prints today. It still lists at most 20 paths and names `.decree/attention/`.
2. The run that found the leftovers writes the same lines to `$DECREE_RUN_DIR/leftovers`; decree includes that file's text in the run's attention file (migration 106), as it does `hold`. decree itself runs no git command.
3. Template and this repository's copy stay identical (`templates_test`). `docs/reference/scripts.md` (built-in scripts) and the skill's built-in machines paragraph describe it. CHANGELOG under Changed.

- Only this migration's scope.
- Never edit `.decree/migrations/`, `.decree/runs/` or `.decree/processed.md`, not even by a search and replace across the repository: exclude them from every bulk edit.
- If the reference docs and this migration disagree in a way you cannot settle, write the question to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- No test calls a real model or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass, and `decree check` passes in every example directory.

## Acceptance Criteria

- **Given** a `develop` run interrupted after its implement step changed files, and a second message queued
  **When** `process` runs the second
  **Then** `clean_tree` fails, and its output names the interrupted run with both its `decree process --retry <id>` and `git stash push --include-untracked -m "decree <id>" -- .` commands

- **Given** that the person runs the printed stash command
  **When** `process` runs again
  **Then** the second message's run passes `clean_tree`

- **Given** changes no run left (a person's own edits)
  **When** `clean_tree` runs
  **Then** it prints today's message and names no run
