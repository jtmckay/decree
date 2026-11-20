---
routine: rust-develop
---
# 63: v0.5 cleanup: no configuration file

## Overview

`config.yml` is down to settings that are better as conventions or fixed limits. Remove it, so a project is only machines, scripts and messages. The spec already describes this (`docs/0.5-spec.md` section 3, "No configuration file"), and `mock/` already has no `config.yml` and a router machine named `router`.

## Requirements

Read `docs/0.5-spec.md` sections 3 (No configuration file), 4 (Frontmatter keys, M1–M3), 5 (Invoke, V16), 6 (Resolution, Attempts, Execution), 7 (Choose: model, The default router), 8 (`decree init`) and 11 (M5.4); `mock/` (no `config.yml`, `machines/router.yml`, `scripts/router/ask_claude.sh`, `mock/README.md`); then `src/config.rs` and every use of it.

1. Delete `src/config.rs` and the config type. Replace each setting:
   - a `choose: model` with no `router:` uses the machine named `router` (V16 checks it exists when needed);
   - `machine:` (or `routine:`) is required on every message: M1–M3 fail a message without one;
   - `max_attempts` comes from the state, default 1; give every state in `src/templates/` and `examples/` that relied on the old default of 3 an explicit `max_attempts`;
   - `max_depth` is the constant 10, and the script log cap is the constant 2097152 bytes;
   - shared sources are gone: script resolution is `scripts/<machine>/` then `scripts/`, and machines load only from `machines/`.
2. Any command run in a project that has `.decree/config.yml` exits 1 with `.decree/config.yml is not used by decree 0.5; run scripts/migrate-0.4-to-0.5.sh`.
3. `decree init` writes no `config.yml`. It writes `machines/router.yml` with `scripts/router/ask_<ai>.sh` for the `--ai` backend (claude, copilot or opencode), replacing `claude_router`, `copilot_router` and `opencode_router`. Its templates for the router match `mock/.decree/machines/router.yml` and `mock/.decree/scripts/router/ask_claude.sh`.
4. `scripts/migrate-0.4-to-0.5.sh` moves `config.yml` into `.decree/legacy-0.4/` with the other removed paths, and also lists every pending message that names no machine.
5. Update `examples/` (remove each `config.yml`; add `machine:` where a message relied on the default; rename router machines to `router`), `README.md`, the decree skill (`src/templates/skills/`, `.claude/skills/`, `.github/skills/`), `docs/routers.md`, `docs/services.md` and the tests.
6. The docs migration that runs next already wrote drafts of `docs/reference/README.md` and `docs/reference/messages.md` that describe `config.yml`. Leave them; that migration revises them.

- Only this migration's scope; migrations 64–68 cover the rest of the cleanup.
- If the code does something the spec does not cover, or the spec is ambiguous here, do not guess: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes. The routine fails the run when `STOP` exists.
- Tests build their own `.decree/` in a temp directory and never touch this repository's `.decree/`. No test calls a real LLM or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass. The routine checks this itself after you finish.

## Files to Modify

- src/config.rs — delete
- src/ (every use of the config)
- src/templates/ (router, no config.yml, explicit max_attempts)
- scripts/migrate-0.4-to-0.5.sh
- examples/, README.md, docs/routers.md, docs/services.md, skills
- tests/

## Acceptance Criteria

- **Given** a fresh temp directory
  **When** `decree init --ai claude` runs, then `decree check`
  **Then** there is no `.decree/config.yml`, `machines/router.yml` and `scripts/router/ask_claude.sh` exist, and check exits 0

- **Given** `decree init --ai copilot` and `--ai opencode`
  **When** they run
  **Then** each writes `machines/router.yml` with `scripts/router/ask_copilot.sh` or `ask_opencode.sh`

- **Given** a project with a `.decree/config.yml`
  **When** `decree check`, `decree process` or `decree status` runs
  **Then** it exits 1 with the message in item 2

- **Given** an inbox message, a migration and a cron file without `machine:`
  **When** `decree check` runs
  **Then** it fails M2, M1 and M3 for them

- **Given** a `choose: model` without `router:` in a project with no machine named `router`
  **When** `decree check` runs
  **Then** it fails V16

- **Given** `mock/` and every project in `examples/`
  **When** `decree check` and `decree graph` run
  **Then** check exits 0, and graph reproduces the committed `.decree/graph/` byte for byte

- **Given** the repository
  **When** `rg -n --hidden 'config\.yml|default_router|default_machine|shared_source|claude_router' --glob '!.decree/migrations/**' --glob '!docs/0.5-spec.md' --glob '!docs/reference/**'` runs
  **Then** it matches only the migration script's handling of 0.4 configs, the item 2 message and their tests
