---
machine: develop
---
# A07: Bare `decree` prints a synopsis

## Overview

Bare `decree` is `decree process` now. Someone who types `decree` to see what the tool does starts agents and commits instead. Most CLIs print help when given no command (git, cargo, docker). The user wants bare `decree` to print a short synopsis of the README's quick start: install or update, init, where a script, a machine and a message go, and the main commands. The text is `src/templates/synopsis.md` in this repository, written by the user. decree prints it as it is.

## Outcome

- `decree` with no command prints `src/templates/synopsis.md` byte for byte and exits 0, inside a project or not. It reads, writes and runs nothing.
- `decree process` is unchanged. Nothing else runs `process` implicitly any more.
- Every place that says bare `decree` is `decree process` says what it does now instead: the reference docs, help, the README and the skill.

## Constraints

- Every `decree` command the synopsis names exists. A test fails when one does not.
- The install line in the synopsis is the README's, and the one `decree upgrade` prints. A test fails when they differ.
- The synopsis is the user's text. Change it only where a test above finds it wrong.
- Only this migration's scope.
- Never edit `.decree/migrations/`, `.decree/runs/` or `.decree/processed.md`, not even by a search and replace across the repository: exclude them from every bulk edit.
- If the reference docs and this migration disagree in a way you cannot settle, write the question to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- No test calls a real model or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass, and `decree check` passes in every example directory.
- CHANGELOG under Changed, and a decision in `docs/decisions.md` saying why.

## Acceptance Criteria

- **Given** a directory with no `.decree/`
  **When** `decree` runs
  **Then** it prints the synopsis and exits 0

- **Given** a project with a pending migration
  **When** `decree` runs
  **Then** it prints the synopsis, exits 0, and the migration is still pending with no run started

- **Given** the synopsis
  **When** the tests run
  **Then** each `decree <command>` it names is one of decree's commands

- **Given** the synopsis, the README and `decree upgrade`
  **When** the tests run
  **Then** they pass only while all three show the same install line
