---
machine: develop
---
# 108: `process` and `daemon` run only the machines they are given

## Overview

A beta user wants several decree processes in one project folder, each handling its own machines (one runs `develop`, another `comfy`). Processes already stay apart: an inbox claim is an atomic rename and a run has its `.lock` ([messages.md](../../docs/reference/messages.md), Claim and Run lock). What is missing is a way to say which machines a process runs. The user chose an allow-list or a deny-list, by flag, shell variable or `.decree/.env`. Prior art: Celery's `--queues` and `--exclude-queues`.

## Requirements

1. **Flags.** `process` and `daemon` take `--machine <m>` or `--exclude-machine <m>`, each repeatable and each taking a comma-separated list. Both together: exit 2. A name that is not a machine in `.decree/machines/` (or a built-in): an error naming it, exit 1. `--dry-run` and `--retry` honour the filter (`--retry <id>` of a run for an excluded machine: refused, exit 1, naming the machine).
2. **Variables.** `DECREE_MACHINES` and `DECREE_EXCLUDE_MACHINES` (comma-separated) set the same, from the shell or `.decree/.env`; a flag beats both, the shell beats `.env`. `dotenv::is_reserved` lets exactly these two keys through in `.decree/.env`; every other `DECREE_*` key is still rejected. decree reads them itself and never passes them to scripts. Both set: exit 2.
3. **What a filtered process does.** It delivers replies and timeouts, continues `pending` runs, claims inbox messages and fires cron only for its machines; everything else stays where it is, untouched. Migrations keep one order for every process: when the first pending migration is for a machine it does not run, it waits, starting no later migration, and ends with ``Held back: migration <file> is for machine `<m>`, which this process does not run`` (exit 0, like the other `Held back:` lines). A child run belongs to its parent: a process that runs the parent steps its children whatever their machine.
4. **Docs.** cli.md rows for the flags and variables; a "Several processes" section under "`process` or `daemon`": the lock and the inbox rename keep processes apart; `.decree/.env` applies to every process in the folder, so give different filters by flag or shell; the built-in `develop`, `infra` and `qa` share the work tree through `clean_tree` and `commit`, so two processes must not run them at once in one checkout (that is the user's job; use a git worktree per process); an attention file with `holds: true` (migration 106) holds every process, whatever its filter. Update D31 ("one run at a time per process") with a pointer, and add D69 (filters; global migration order; why no leases).
5. **Tests**: the ACs below, with two `process` commands on one project.
6. **The skill** (`src/templates/skills/decree/SKILL.md`, then `decree skill` refreshes `.claude/skills/decree/`): the `process` and `daemon` rows name `--machine` and `--exclude-machine`; one sentence in the built-in machines paragraph: never run `develop`, `infra` or `qa` in two processes in one checkout (they share the work tree through `clean_tree` and `commit`); give each its own git worktree.

- Only this migration's scope.
- Never edit `.decree/migrations/`, `.decree/runs/` or `.decree/processed.md`, not even by a search and replace across the repository: exclude them from every bulk edit.
- If the reference docs and this migration disagree in a way you cannot settle, write the question to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- No test calls a real model or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass, and `decree check` passes in every example directory.

## Acceptance Criteria

- **Given** inbox messages for machines `a` and `b`
  **When** `decree process --machine a` runs
  **Then** only `a`'s messages ran and `b`'s are still in `inbox/`; then `decree process --exclude-machine a` runs `b`'s

- **Given** `DECREE_MACHINES=a` in `.decree/.env`
  **When** `decree process` runs, then `decree process --machine b`
  **Then** the first runs only `a`, the second only `b`, and a script that prints its environment shows no `DECREE_MACHINES`

- **Given** both `--machine a` and `--exclude-machine b`
  **When** `process` runs
  **Then** it exits 2 and runs nothing

- **Given** pending migrations `01` for `b` and `02` for `a`
  **When** `decree process --machine a` runs
  **Then** neither runs, it prints ``Held back: migration 01… is for machine `b` `` and exits 0

- **Given** `DECREE_FOO=1` in `.decree/.env`
  **When** `decree check` runs
  **Then** it still rejects the key
