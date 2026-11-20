---
routine: rust-develop
---

# 31: Docs and help refresh (migrations 18–30)

## Overview

Update `src/templates/help.txt`, `README.md`, and `SOW.md` to reflect every
feature introduced in migrations 18 through 30. No code changes — docs and
help text only.

## Features to Document

### Migration 18 — YAML array/object env vars
Custom frontmatter fields whose values are YAML arrays or mappings are now
serialized as compact JSON strings and passed as env vars to routines and hooks.

### Migration 19 — Inbox filename normalization
When `normalize()` generates a new chain ID, it uses the inbox file's own
stem as the name component instead of the literal string `"message"`. The
inbox file is also renamed to match the generated message ID.

### Migration 20 — DECREE_FINAL_ATTEMPT
`DECREE_FINAL_ATTEMPT=true` is set in the `afterEach` environment when the
current attempt is the last allowed attempt (attempt == max_retries). The
variable is absent on non-final attempts.

### Migration 21 — run.json per-run metadata
After processing completes (success or failure), Decree writes `run.json`
to the run directory. Fields: `message_id`, `routine`, `trigger`, `migration`
(if applicable), `attempts`, `exit_code`, `start`, `end`, `duration_s`.

### Migration 22 — Per-routine max_retries and timeout_s
Individual routines can override the global `max_retries` and add a
`timeout_s` cap. Example:
```yaml
routines:
  gmail-sync:
    enabled: true
    max_retries: 5
  actual-budget:
    enabled: true
    timeout_s: 60
```
When `timeout_s` is set, the process is killed with SIGTERM after that many
seconds and the attempt is treated as exit code 1.

### Migration 23 — onDeadLetter hook
New `onDeadLetter` hook fires exactly once when a message is moved to
`inbox/dead/` after exhausting all retries. It does NOT fire on `beforeEach`
failures. Additional env vars: `DECREE_ROUTINE_EXIT_CODE`, `DECREE_TRIGGER`.

### Migration 24 — DECREE_TRIGGER
`DECREE_TRIGGER` is set in routine and hook environments to track how a run
was initiated:
- `inbox` — manually dropped or created from a migration
- `cron:<stem>` — fired by the cron daemon from a file with the given stem
- `chain` — promoted from outbox as a follow-up message

Also included in `run.json`.

### Migration 25 — decree status dead-letter timestamp
When at least one dead-letter file exists, `decree status` shows the oldest
file's modification time: `Dead-lettered: 3 messages  (oldest: 2026-04-26T02:19:34)`.

### Migration 26 — decree cron list
New `decree cron list` command prints a table of all cron files with their
schedule, inferred routine, relative last-run age (`2m ago`, `never`), and
countdown to next fire (`13m`, `—`).

### Migration 27 — decree skill
New `decree skill` command installs Decree guidance for Claude or GitHub
Copilot:
- `decree skill --scope project --target claude` → `.claude/skills/decree/SKILL.md`
- `decree skill --scope project --target copilot` → `.github/copilot-instructions.md`
- `decree skill --scope user --target claude` → `~/.claude/skills/decree/SKILL.md`
- `decree skill --scope user --target copilot` → not supported (exits non-zero)
- `--force` overwrites an existing file that differs from the bundled template
- Idempotent: no-ops if the installed file already matches the bundle

### Migration 28 — Stop processing on migration dead-letter
When a migration's inbox message is dead-lettered (all retries exhausted),
`decree process` stops immediately and exits non-zero. Subsequent migrations
are not started. Inline inbox drains (non-migration messages) are unaffected.

### Migration 29 — Claude token exhaustion: detect, wait, retry
After a non-zero routine exit, Decree scans the run log for the pattern
`usage limit` + `reset`. If found:
1. Parses the reset time (falls back to +1 hour if unparseable)
2. Prints a waiting message to stderr
3. Sleeps until reset (SIGINT-aware: exits code 130 on Ctrl+C)
4. Removes the migration from `processed.md` and any dead-letter copy
5. Re-enters the migration loop — the migration is retried from scratch

### Migration 30 — Resume Claude session on token-exhaustion retry
After a token-exhaustion wait, Decree passes the previous Claude session ID
to the retry attempt via `DECREE_PREVIOUS_SESSION_ID`. The session ID is
extracted from the run log pattern `Session ID: <id>` and persisted as
`session_id.txt` in the run directory. Routines opt in:
```bash
resume_flag=""
if [ -n "${DECREE_PREVIOUS_SESSION_ID:-}" ]; then
  resume_flag="--resume $DECREE_PREVIOUS_SESSION_ID"
fi
claude $resume_flag -p "$prompt"
```
The default `develop.sh` and `rust-develop.sh` templates use this pattern.

## Requirements

### src/templates/help.txt

Update the following sections:

**Commands table** — add:
```
  decree cron list            List cron schedules with last/next run times
  decree skill                Install AI assistant skill/instructions
```

**Environment Variables** — add to "Hook-only env vars":
```
  DECREE_FINAL_ATTEMPT        "true" on final retry attempt (afterEach only)
  DECREE_TRIGGER              How the run was initiated: inbox | cron:<stem> | chain
```

Add a new subsection "Retry env vars (set on token-exhaustion retry)":
```
  DECREE_PREVIOUS_SESSION_ID  Claude session ID from the prior attempt, if captured
```

**Lifecycle Hooks** — add `onDeadLetter` to the hooks config example and description.

Add new env vars available to `onDeadLetter`:
```
  DECREE_ATTEMPT              Equals max_retries (all retries were exhausted)
  DECREE_MAX_RETRIES          Configured max retries
  DECREE_ROUTINE_EXIT_CODE    Exit code of the last attempt
  DECREE_TRIGGER              How the run was initiated
```

**Routine Registry** — add per-routine `max_retries` and `timeout_s` to the
config layout example and describe their behavior.

**Processing Pipeline** — update step 6 (success path) to mention `run.json`
is written after processing. Update step 8 (dead-letter) to mention that
migration dead-letters stop the loop and exit non-zero. Add a note about
token-exhaustion detection and automatic retry-after-wait.

**Cron Scheduling** — add a note that `decree cron list` shows live schedule
status.

Add a new **run.json** section describing the machine-readable record written
to each run directory, listing all fields.

Add a new **AI Skill Installation** section describing `decree skill` with
scope/target options and the `--force` flag.

### README.md

Update or add the following sections:

**Lifecycle Hooks** — extend the hooks example to include `onDeadLetter`.
Document `DECREE_FINAL_ATTEMPT` and `DECREE_TRIGGER` env vars.

**Retry & Dead-Letter** — add a subsection describing:
- Per-routine `max_retries` and `timeout_s` config
- Migration dead-letters stop the process loop (`decree process` exits non-zero)
- Token exhaustion: auto-detect, wait until reset, retry with session resume

**run.json** — add a short section or callout describing the machine-readable
metadata file written to each run directory.

**decree cron list** — add a subsection under Daemon & Cron showing the
output format.

**decree skill** — add a new section "AI Assistant Integration" describing
the `decree skill` command, its scope/target options, and what gets installed.

**Project Structure** — add `run.json` as a noted artifact inside run
directories.

### SOW.md

**Scope — In scope** — add:
- Per-routine `max_retries` and `timeout_s` configuration overrides
- `onDeadLetter` lifecycle hook
- `DECREE_TRIGGER`, `DECREE_FINAL_ATTEMPT`, and `DECREE_PREVIOUS_SESSION_ID` env vars
- Machine-readable `run.json` written to each run directory
- `decree cron list` command for live schedule inspection
- `decree skill` command for AI assistant integration (Claude and Copilot)
- Claude token-exhaustion detection with automatic wait-and-retry
- Claude session resume (`DECREE_PREVIOUS_SESSION_ID`) after token-exhaustion wait
- Migration dead-letter stops the process loop and exits non-zero

**Deliverables** — expand item 5 (lifecycle hook system) to mention
`onDeadLetter`; expand item 6 (daemon) to mention `decree cron list`; add a
new deliverable for AI skill installation.

**Acceptance Criteria** — add:
- Per-routine `max_retries` overrides the global value for that routine; `timeout_s` kills the process after the given seconds
- `onDeadLetter` hook fires exactly once when a message exhausts retries; does not fire on `beforeEach` failure
- `DECREE_TRIGGER` is set in routines and all hook phases to `inbox`, `cron:<stem>`, or `chain`
- `DECREE_FINAL_ATTEMPT=true` is present in `afterEach` on the last attempt only
- `run.json` is written to the run directory after every completed run (success or dead-letter)
- `decree cron list` shows all cron files with schedule, last-run age, and next-fire countdown
- `decree skill --scope project --target claude` installs `.claude/skills/decree/SKILL.md`
- `decree skill --scope project --target copilot` installs `.github/copilot-instructions.md`
- Token-exhaustion detection pauses processing, waits until the reset time, then retries the migration
- SIGINT during a token-exhaustion wait exits with code 130
- A migration that is dead-lettered stops `decree process` immediately; subsequent migrations do not run

## Files to Modify

- `src/templates/help.txt` — add new commands, env vars, sections as above
- `README.md` — update Lifecycle Hooks, Daemon & Cron, Retry sections; add decree skill, run.json
- `SOW.md` — update Scope, Deliverables, and Acceptance Criteria

## Acceptance Criteria

- **Given** the updated `help.txt`
  **When** `decree help` is run
  **Then** `decree cron list` and `decree skill` appear in the Commands table

- **Given** the updated `help.txt`
  **When** a reader looks up env vars
  **Then** `DECREE_FINAL_ATTEMPT`, `DECREE_TRIGGER`, and `DECREE_PREVIOUS_SESSION_ID`
  are documented with accurate descriptions

- **Given** the updated `help.txt`
  **When** a reader looks up lifecycle hooks
  **Then** `onDeadLetter` is listed with its available env vars and the note
  that it does not fire on `beforeEach` failure

- **Given** the updated `help.txt`
  **When** a reader looks up the Routine Registry section
  **Then** per-routine `max_retries` and `timeout_s` are shown in the config
  example and described

- **Given** the updated `help.txt`
  **When** a reader looks up the Processing Pipeline section
  **Then** `run.json` is mentioned, migration dead-letters stopping the loop
  is mentioned, and token-exhaustion retry behavior is described

- **Given** the updated `help.txt`
  **When** a reader looks up Cron Scheduling
  **Then** `decree cron list` is mentioned as the command for live schedule status

- **Given** the updated `README.md`
  **When** a reader reads the Lifecycle Hooks section
  **Then** `onDeadLetter` is present in the hooks config example and
  `DECREE_TRIGGER` / `DECREE_FINAL_ATTEMPT` are documented

- **Given** the updated `README.md`
  **When** a reader reads about retries
  **Then** per-routine overrides, migration dead-letter stopping the loop,
  and token-exhaustion auto-retry with session resume are all described

- **Given** the updated `README.md`
  **When** a reader reads the Daemon & Cron section
  **Then** `decree cron list` is documented with its output format

- **Given** the updated `README.md`
  **When** a reader reads about AI assistant integration
  **Then** `decree skill` is documented with project/user scope and claude/copilot target options

- **Given** the updated `SOW.md`
  **When** a reader reads the In Scope list
  **Then** all features from migrations 18–30 are listed

- **Given** the updated `SOW.md`
  **When** a reader reads Acceptance Criteria
  **Then** all behaviorally verifiable criteria for migrations 18–30 are present
