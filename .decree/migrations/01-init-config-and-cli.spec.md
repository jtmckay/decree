# 01: Init, Config, and CLI

## Overview

Decree 0.2.0

`decree init` scaffolds the project layout and detects available AI backends.
The config schema, CLI definition, main dispatch, and error types all live here.

## Directory Creation

`decree init` creates:

```
.decree/
├── config.yml
├── .gitignore
├── router.md               # Internal: routine selection prompt
├── routines/
│   ├── develop.sh
│   └── rust-develop.sh
├── prompts/                # User prompt templates
│   ├── migration.md
│   ├── sow.md
│   └── routine.md
├── cron/
├── inbox/
│   └── dead/
├── outbox/
│   └── dead/
├── runs/
├── migrations/
└── processed.md            # Empty tracker file
```

## Re-Run Behavior

If `.decree/` already exists when `decree init` is run:

```
Decree is already configured in this directory.
Overwrite existing configuration? [y/N]
```

Default is **No**. If declined, exit with no changes. If accepted,
proceed with normal init (overwriting existing files).

## AI Backend Detection

During `decree init`, auto-detect available AI tools in this order:

1. `opencode` — check `which opencode`
2. `claude` — check `which claude`
3. `copilot` — check `which copilot`

If multiple are found, present an arrow-key selector. If none are found,
print a message suggesting https://opencode.ai/ as a path to install
opencode, and set the command to `opencode` (user can install later).

The selected tool populates `commands.ai_command` in config.yml.
The default routine templates use `{AI_CMD}` placeholders which are
replaced at init time with the detected AI command name.

## Git Detection and Lifecycle Hooks

After AI backend selection, check if `git` is installed (`which git`)
and if the project is inside a git repo (`git rev-parse --is-inside-work-tree`).

**If git is detected:**

```
Enable git stash hooks for change tracking? [Y/n]
```

Default is **Yes**. If accepted:

- Copy `git-baseline.sh` and `git-stash-changes.sh` to `.decree/routines/`
- Uncomment the `beforeEach` and `afterEach` git stash hook lines in config.yml

If declined, git stash hook lines stay commented out.

**If git is not detected:**

- Print: `git not found — skipping lifecycle hook setup`
- Git stash hook lines stay commented out
- No git-related routines are created

In either case, config.yml always contains the commented-out git stash
workflow so users can enable it later by uncommenting.

## Config File (`config.yml`)

```yaml
commands:
  ai_command: "opencode" # AI tool command name
  ai_router: "{ai_command} run {prompt}" # AI for routine selection
  ai_interactive: "{ai_command}" # Interactive AI session
  # ai_command: "claude"
  # ai_router: "claude -p {prompt}"
  # ai_interactive: "claude"
  # ai_command: "copilot"
  # ai_router: "copilot {prompt}"
  # ai_interactive: "copilot"

max_retries: 3
max_depth: 10
max_log_size: 2097152 # Per-log size cap in bytes (2MB), 0 to disable
default_routine: develop

hooks:
  beforeAll: "" # Routine name to run before all processing
  afterAll: "" # Routine name to run after all processing
  beforeEach: "" # Routine name to run before each message
  afterEach: "" # Routine name to run after each message
  # --- Git stash workflow (uncomment to enable) ---
  # beforeEach: "git-baseline"
  # afterEach: "git-stash-changes"
```

Fields:

- `commands.ai_command` — the AI tool command name (e.g. `opencode`, `claude`)
- `commands.ai_router` — AI command template for routine selection (`{prompt}` is replaced)
- `commands.ai_interactive` — interactive AI session launched by `decree prompt`
- `max_retries` — per-message retry limit (default: 3)
- `max_depth` — inbox recursion limit (default: 10)
- `max_log_size` — per-log file size cap in bytes (default: 2097152 = 2MB, 0 to disable)
- `default_routine` — fallback routine name (default: "develop")
- `hooks.*` — lifecycle hook routine names (see migration 10)

## Message ID Format

A message ID has the form `<chain>-<seq>`:

- **Day counter** `D<NNNN>`: 4-digit counter that ensures chronological
  ordering. Resolution: find the highest existing day counter in
  `.decree/runs/`, then compare the current `HHmm` to the last entry's
  `HHmm`. If current `HHmm` >= last entry's `HHmm`, reuse the same day
  counter. If current `HHmm` < last entry's `HHmm` (clock wrapped past
  midnight), increment the day counter. First run starts at `D0001`.
- **Chain ID**: `D<NNNN>-HHmm-<name>` where `HHmm` is the time (hours and
  minutes) and `<name>` is the full migration filename stem
  (e.g., `01-add-auth` from `01-add-auth.md`) or the routine name for
  ad-hoc runs (e.g., `develop`).
  Generated once for the root message; all messages in a chain share it.
- **Sequence number**: starts at `0`, decree increments by 1 when collecting
  outbox follow-ups.

Examples:
- `D0001-1432-01-add-auth-0` — first message for migration `01-add-auth.md`
- `D0001-1432-01-add-auth-1` — follow-up in the same chain
- `D0001-1435-02-add-database-0` — second migration, same day
- `D0002-0900-03-add-api-0` — next day, new day counter

Commands accept full IDs, chain IDs, or unique prefixes.

## Full CLI

```
decree                                # Smart default (process)
decree init                           # Initialize project
decree process [--dry-run]             # Process all migrations + drain inbox
decree prompt [NAME]                  # Build prompt, copy or launch AI
decree routine                        # List routines (interactive)
decree routine <name>                 # Show routine detail + run pre-checks
decree verify                         # Run all routine pre-checks
decree daemon [--interval SECS]       # Daemon: monitor inbox + cron
decree status                         # Show progress
decree log [ID]                       # Show execution log
decree help                           # Verbose help (see migration 12)
decree --version / -v                 # Print version and exit
```

## Main Dispatch

`main.rs` must:

- Check for `.decree/` existence (except for `init` and `help`)
- Dispatch to all command handlers
- Handle `decree daemon` lifecycle (signal handling)
- Default bare `decree` to `decree process`

## Error Types

`DecreeError` variants:

- `RoutineNotFound` — referenced routine doesn't exist
- `MaxRetriesExhausted` — all retries for a message failed
- `MaxDepthExceeded` — inbox recursion limit hit
- `NoMigrations` — no migration files found
- `MessageNotFound` — referenced message ID doesn't exist
- `PreCheckFailed` — routine pre-check failed

## `decree status`

Displays a summary of project state:

```
Migrations:
  Processed: 3 of 5
  Next: 04-add-notifications.md

Inbox:
  Pending: 2 messages
  Dead-lettered: 1 message

Recent Activity (last 5):
  D0001-1432-01-add-auth-0  develop       done   01-add-auth.md
  D0001-1432-01-add-auth-1  rust-develop  done   (follow-up)
  D0001-1435-02-add-database-0  develop   done   02-add-database.md
  D0002-0900-03-add-api-0   develop       dead   03-add-api.md
  D0002-0900-03-add-api-1   develop       dead   (follow-up)
```

Sections:

- **Migrations**: count of processed vs total in `.decree/migrations/`,
  and the next unprocessed migration filename
- **Inbox**: count of pending messages in `.decree/inbox/` and
  dead-lettered messages in `.decree/inbox/dead/`
- **Recent Activity**: last 5 messages from `.decree/runs/`, showing
  message ID, routine name, disposition (done/dead), and migration
  name or "(follow-up)" for non-root messages. Sorted by directory
  name (which is chronological by chain ID).
  Disposition logic: run dir exists and not in `inbox/dead/` = done;
  in `inbox/dead/` = dead.

**Non-TTY**: print with no ANSI formatting.

## `decree log`

Reads log files from message directories in `.decree/runs/`.

- **No args (TTY)**: arrow-key selector of recent runs (most recent first).
- **No args (Non-TTY)**: print the most recent run's log.
- **With ID (unique match)**: show that run's logs.
- **With ID (ambiguous, TTY)**: arrow-key selector of matching runs.
- **With ID (ambiguous, Non-TTY)**: list all matching logs.

If multiple attempts exist (`routine.log`, `routine-2.log`, etc.), all
are shown with attempt headers.

## Non-TTY Behavior

| Command | Non-TTY Behavior |
|---|---|
| `decree init` | Auto-detect AI, accept git hooks if detected, error if unresolvable |
| `decree process` | Print status line per migration, no ANSI progress bar |
| `decree status` | Print to stdout, no ANSI formatting |
| `decree log` | Print most recent log / matching logs to stdout |
| `decree routine` | Print list and exit (spec 05) |
| `decree prompt` | Print list and exit (spec 09) |
| `decree daemon` | Runs normally, log to stdout |
| `decree verify` / `decree help` | Already non-interactive |

## Exit Code Conventions

| Code | Meaning |
|---|---|
| 0 | Success |
| 1 | General failure (routine failed, command error) |
| 2 | Usage error (bad args, unknown subcommand) |
| 3 | Pre-check failure (`decree verify`, `decree process --dry-run`) |

## Color Conventions

- **Green**: success / PASS / done
- **Red**: errors / FAIL / dead
- **Yellow**: warnings / skipped
- **Bold**: headers / command names
- **Dim**: timestamps / paths

Color is disabled when:
1. `--no-color` flag is passed (highest precedence)
2. `NO_COLOR` env var is set (per [no-color.org](https://no-color.org))
3. stdout is not a TTY (auto-detect)

Precedence: `--no-color` > `NO_COLOR` > auto-detect.

## Acceptance Criteria

- [ ] `decree init` creates `.decree/` with subdirs: `routines/`, `prompts/`,
      `cron/`, `inbox/` (with `dead/`), `outbox/` (with `dead/`), `runs/`
- [ ] `.decree/migrations/` and `.decree/processed.md` are created
- [ ] `.decree/router.md` is created (not in `prompts/`)
- [ ] Re-running `decree init` warns and asks to overwrite (default No)
- [ ] AI backend is auto-detected from opencode/claude/copilot
- [ ] If no AI backend found, suggests opencode.ai and sets `opencode` as default
- [ ] `commands.ai_command` is set in config based on detected AI
- [ ] Git is detected via `which git` and `git rev-parse --is-inside-work-tree`
- [ ] If git detected, user is offered git stash hooks (default Yes)
- [ ] If git hooks accepted, hook routines are created and config lines uncommented
- [ ] If git hooks declined or git not found, git stash hook lines stay commented out
- [ ] `config.yml` includes all AI backends (selected uncommented, others commented out)
- [ ] `config.yml` includes commented-out git stash workflow for easy enablement
- [ ] All CLI subcommands are accepted and dispatched correctly
- [ ] `decree status` shows migrations progress, inbox counts, and recent activity
- [ ] `decree log` displays routine output for specified message
- [ ] `decree log` without ID (TTY) shows arrow-key selector of recent runs
- [ ] `decree log` without ID (Non-TTY) shows most recent run's log
- [ ] `decree log` with ambiguous ID (TTY) shows arrow-key selector of matches
- [ ] `decree log` with ambiguous ID (Non-TTY) lists all matching logs
- [ ] Bare `decree` dispatches to `decree process`
- [ ] Exit codes follow convention: 0 success, 1 failure, 2 usage error, 3 pre-check failure
- [ ] Color output uses green/red/yellow/bold/dim conventions
- [ ] `--no-color` flag disables color output
- [ ] `NO_COLOR` env var disables color output
- [ ] Color auto-detects TTY when no flag or env var is set
- [ ] Non-TTY behavior is correct for all commands per the table
