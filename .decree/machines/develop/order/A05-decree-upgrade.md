---
machine: develop
---
# A05: `decree upgrade` prints how to upgrade

## Overview

To update decree, you run the install line again, but nothing in decree says so. You have to find the line in the README. The user wants one command that prints it. Like `rustup self update` and `uv self update`, it is reached from the binary itself. Unlike them, it only prints the line and leaves running it to the person, so decree makes no network call. It also has to point to the right route for an install that came from `cargo install`, since the install line does not replace that copy.

## Outcome

- `decree upgrade` prints the following and exits 0. It works with or without a `.decree/` folder:
  - the running version (`decree 0.5.0-beta.13`);
  - the install line for the newest release, exactly as the README gives it;
  - the same line pinned to a version (`| sh -s -- <tag>`);
  - for a copy installed with `cargo install`: `cargo install --git https://github.com/jtmckay/decree --tag <tag> --force`, and a note that when `which -a decree` lists two copies, the one first on `PATH` runs, so remove the other (`cargo uninstall decree`);
  - what to do afterwards in each project: `decree skill`, plus `decree schema` if the project has `.decree/schema/`.
- `decree upgrade --format json` prints one document with the same content, described by `schema/v1/cli/upgrade.schema.json` (written by `decree schema` like the others).
- The command is listed wherever commands are listed: the README commands table and its `--format json` list, `docs/reference/cli.md` (its command count, plus a pointer from "Upgrading decree"), help, and the skill's Commands table and `--format json` list.

## Constraints

- decree makes no network call and runs no other program for this command: it only prints.
- The install URL appears once in the source. A test fails if the line that `decree upgrade` prints differs from the README's install line, so that when `v0.5` becomes `main` in the URL, both change together.
- Only this migration's scope.
- Never edit `.decree/migrations/`, `.decree/runs/` or `.decree/processed.md`, not even by a search and replace across the repository: exclude them from every bulk edit.
- If the reference docs and this migration disagree in a way you cannot settle, write the question to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- No test calls a real model or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass, and `decree check` passes in every example directory.
- CHANGELOG under Added.

## Acceptance Criteria

- **Given** an empty directory
  **When** `decree upgrade` runs
  **Then** it exits 0 and prints the running version, the README's install line, the pinned form, the `cargo install --git … --force` line with the `which -a decree` note, and `decree skill`

- **Given** the README
  **When** the tests run
  **Then** a test checks that the README's install line appears verbatim in `decree upgrade`'s output

- **Given** any directory
  **When** `decree upgrade --format json` runs
  **Then** its output validates against `schema/v1/cli/upgrade.schema.json`

- **Given** the README, cli.md, help and the skill
  **When** `tests/readme_test.rs` and the docs tests run
  **Then** they pass with `upgrade` among the listed commands
