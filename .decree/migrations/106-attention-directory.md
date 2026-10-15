---
machine: develop
---
# 106: `.decree/attention/` says what needs a person, and replaces `.decree/HOLD`

## Overview

A beta user lost hours to runs nobody knew were blocked: beta.9 exited silently, a run sat in a `person` state for 6.5 hours, and beta.10's message is the last line of a long log. `.decree/HOLD` (D65) covers one case only, and the user has yet to see it help. The user chose: decree keeps one file per run that needs a person, in `.decree/attention/`, the way `processed.md` records what is done, and that folder replaces HOLD. The files are read-only: answers still go through `decree event` or a reply message (D40), and a run is still continued with `decree process --retry`. No hook: a `person` state's `ask` script already tells someone.

## Requirements

1. **When decree writes a file.** `.decree/attention/<id>.md` is written when run `<id>` becomes `failed`, `interrupted` (a signal, or a crash found later from a stale lock), or `waiting` in a `person` state; and for a migration that blocks the queue by waiting for an interrupted child (beta.10's chain), under the migration's id, naming the child. It is deleted when the run becomes `active` or `pending`, finishes in a state other than `failed`, or is deleted by `decree prune`. decree writes the files when a run's status changes, never by rescanning, so a file a person deleted stays deleted until the status changes again.
2. **The file.** YAML frontmatter, then markdown:
   - frontmatter: `kind: run`, `run`, `machine`, `message` (the migration file name, or `inbox`), `status` (`failed`, `interrupted`, `waiting`), `state`, `since` (RFC 3339 UTC), `holds` (`true` or `false`). A schema `schema/v1/attention.schema.json` (with `$id` like the others, embedded, written by `decree schema`, CHANGELOG in `schema/`).
   - body: a heading naming the message's first heading; what happened and since when; the exact command: `decree process --retry <id>`; for a `person` wait, the question, the options with their descriptions and one `decree event <wait id> <option>` line per option; for a migration, `later migrations are blocked until it finishes`; and, when the run holds, the text of its `hold` file (requirement 3); when the run directory has a `STOP` file (an agent's question, `reference/scripts.md`), its text and `answer it in plan.md, delete STOP, then decree process --retry <id>`.
3. **Holding replaces HOLD.** `keep.sh` writes its message to `$DECREE_RUN_DIR/hold` instead of `.decree/HOLD`. When decree writes the attention file of a run with a `hold` file, it sets `holds: true` and includes the text. While any attention file has `holds: true`, `process` and `daemon` start and continue no runs, exactly as for HOLD today (cli.md "Hold": `process` prints the file and exits 1, `daemon` prints it once and polls; `emit` and `event` still queue). `decree process --retry` of that run removes its `hold` and its attention file. To drop the run instead, a person stashes its changes and deletes the attention file: decree then deletes the run's `hold` (at its next check) and the queue goes on. The manual pause HOLD allowed becomes `decree stop` (migration 107), which writes an attention file of its own.
   - **What counts.** Only `*.md` files directly in `.decree/attention/`; any other file or folder (an editor's swap file, `.DS_Store`) is ignored, holds nothing and is not listed.
   - **Unreadable holds.** A `.md` file that cannot be read, or whose frontmatter is missing, is not YAML, or does not match `attention.schema.json`, holds the queue as if it had `holds: true`. So anything (a person, a script, another tool) can halt the queue by putting any `.md` file there, without knowing the contract. `process` and `status` name such a file with its first line and say why it holds (`no frontmatter`, `invalid frontmatter: <error>`). decree never rewrites or deletes a file it did not write. Only deleting the file releases its hold. Prior art: today's HOLD, which holds even when it cannot be read.
4. **decree points to the folder.** `process` ends, after its existing per-run lines, with `<n> run(s) need attention: .decree/attention/` when the folder has files. `decree status` starts with `Attention:`, the folder, the number of files and the holding run first (this replaces `Held:`); `--format json` adds `attention`, a list of each file's frontmatter plus its path (`schema/v1/cli/status.schema.json`). `clean_tree.sh` and `keep.sh` messages name the folder.
5. **HOLD is gone.** Remove `layout::HOLD_FILE` and every read or write of `.decree/HOLD` from the code, templates (`keep.sh`, `clean_tree.sh`, `.gitignore`: `HOLD` out, `attention/` in), this repository's `.decree/` copies (held to the templates by `templates_test`), the skill and the docs (cli.md "Hold" becomes "Attention", runs.md, messages.md, README). D65 stays in `docs/decisions.md` as history; add D67 (attention folder; supersedes D65's HOLD file; why read-only: D40's one reply channel and the duplicate-reply refusal; why no hook: `ask`, and a killed run cannot run one; why an unreadable `.md` file holds: anything can halt the queue without knowing the contract).
   The files that name HOLD today, every one to change: `src/layout.rs`, `src/commands/{process,daemon,status,init}.rs`; `src/templates/scripts/keep.sh` and `src/templates/gitignore`; the `failed` line's comment in `src/templates/machines/{develop,qa,infra}.yml` (`# the run's changes stay in the tree; its attention file holds the queue`); the copies in this repository's `.decree/` (`machines/{develop,qa,infra}.yml`, `scripts/keep.sh`, `.gitignore`) and every example's `.decree/.gitignore`; `docs/escalation.md`; `README.md`, `docs/reference/{README,cli}.md`; `src/templates/skills/decree/SKILL.md`; the tests in `tests/{integration,cli,develop}_test.rs`.
6. **The skill teaches the attention folder** (`src/templates/skills/decree/SKILL.md`, its `reference/` files, then `decree skill` refreshes this repository's `.claude/skills/decree/`, held identical by `tests/skill_test.rs`):
   - The built-in machines paragraph: `keep` leaves the changes and writes the run's `hold`, so its attention file holds every run. To continue: answer any `STOP` in `plan.md`, delete `STOP`, `decree process --retry <id>`. To drop the run: stash its changes and delete its attention file.
   - A new rule, **What needs a person is in `.decree/attention/`**: read it first when a run is failed, waiting or stuck, and do what each file says (`decree event` for a wait, `decree process --retry` for a failed or interrupted run). Never edit an attention file to answer; never write one from a machine's script. A step whose failure must stop every run writes `$DECREE_RUN_DIR/hold` (what `keep` does), and decree turns it into a holding attention file. Any `.md` file a person or another tool puts in the folder halts the queue until it is deleted.
   - `reference/runs.md` (status and what to do) and `reference/scripts.md` (run-directory files: `hold`) say the same. The Commands table's `status` row says it starts with `Attention:`.
7. **Tests**: each AC below; the attention file validates against its schema; `decree status --format json` validates.

- Only this migration's scope.
- Never edit `.decree/migrations/`, `.decree/runs/` or `.decree/processed.md`, not even by a search and replace across the repository: exclude them from every bulk edit.
- If the reference docs and this migration disagree in a way you cannot settle, write the question to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- No test calls a real model or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass, and `decree check` passes in every example directory.

## Acceptance Criteria

- **Given** a `develop` run that fails with changes in the work tree
  **When** `process` ends
  **Then** `.decree/attention/<id>.md` has `holds: true`, the kept changes and `decree process --retry <id>`; `process` printed `1 run(s) need attention: .decree/attention/`; a second `process` starts nothing and exits 1

- **Given** that held run
  **When** `decree process --retry <id>` runs
  **Then** its attention file and `runs/<id>/hold` are gone before the run continues

- **Given** that held run
  **When** a person deletes the attention file and runs `process`
  **Then** `runs/<id>/hold` is gone and the queue runs

- **Given** a run waiting in a `person` state
  **When** `process` ends
  **Then** its attention file has `holds: false`, the question and one `decree event <wait id> <option>` line per option; after the reply is delivered, the file is gone

- **Given** a run interrupted by SIGINT
  **When** `decree status` runs
  **Then** it starts with `Attention:` and the run's file exists with `status: interrupted`

- **Given** `.decree/attention/notes.md` with no frontmatter, and another with invalid frontmatter
  **When** `process` runs
  **Then** it starts nothing, exits 1 and names each file with why it holds; `decree status --format json` lists both under `attention`; after both are deleted, the queue runs

- **Given** only `.decree/attention/.notes.md.swp` and `.decree/attention/old/x.md`
  **When** `process` runs
  **Then** the queue runs

- **Given** a run that ends in `done`
  **When** `process` ends
  **Then** `.decree/attention/` has no file for it

- **Given** the repository
  **When** `rg -n --hidden '\bHOLD\b' --glob '!.decree/migrations/**' --glob '!.decree/runs/**' --glob '!.git/**' --glob '!CHANGELOG.md' --glob '!docs/decisions.md'` runs
  **Then** it matches nothing
