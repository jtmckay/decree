# 01: Init, Config, and CLI

## Overview

`decree init` scaffolds the project layout and detects available AI backends.
The config schema, CLI definition, main dispatch, and error types all live here.

## Directory Creation

`decree init` creates:

```
.decree/
├── config.yml
├── .gitignore
├── routines/
│   ├── develop.sh
│   └── rust-develop.sh
├── starters/            # Starter prompt templates
│   └── spec.md
├── cron/
├── inbox/
│   ├── done/
│   └── dead/
└── runs/
migrations/              # Adjacent to .decree/, at project root
└── processed.md         # Empty tracker file
```

## AI Backend Detection

During `decree init`, auto-detect available AI tools in this order:

1. `opencode` — check `which opencode`
2. `claude` — check `which claude`
3. `copilot` — check `which copilot`

If multiple are found, present an arrow-key selector. If none are found,
print a message suggesting https://opencode.ai/ as a path to install
opencode, and set the command to `opencode` (user can install later).

The selected tool configures `commands.ai` in config.yml. The default
routine templates use `{AI_CMD}` placeholders replaced at init time.

## Git Detection and Lifecycle Hooks

After AI backend selection, check if `git` is installed (`which git`)
and if the project is inside a git repo (`git rev-parse --is-inside-work-tree`).

**If git is detected:**
```
Enable git stash hooks for change tracking? [Y/n]
```
Default is **Yes**. If accepted:
- Copy `git-baseline.sh` and `git-stash-changes.sh` to `.decree/routines/`
- Set `hooks.beforeEach: "git-baseline"` and `hooks.afterEach: "git-stash-changes"`

If declined, all hook values remain empty strings.

**If git is not detected:**
- Print: `git not found — skipping lifecycle hook setup`
- All hook values remain empty strings
- No git-related routines are created

In either case, hook fields are always present in config.yml (empty or
populated). Users can add or change hooks manually at any time.

## Config File (`config.yml`)

```yaml
commands:
  ai: "opencode run {prompt}"       # AI backend for routine execution
  interactive_ai: "opencode"        # Interactive AI session command

max_retries: 3
max_depth: 10
max_log_size: 2097152          # Per-log size cap in bytes (2MB), 0 to disable
default_routine: develop

hooks:
  beforeAll: ""          # Routine name to run before all processing
  afterAll: ""           # Routine name to run after all processing
  beforeEach: ""         # Routine name to run before each message
  afterEach: ""          # Routine name to run after each message
```

Fields:
- `commands.ai` — AI command template for routine execution (`{prompt}` is replaced)
- `commands.interactive_ai` — interactive AI command launched by `decree starter`
- `max_retries` — per-message retry limit (default: 3)
- `max_depth` — inbox recursion limit (default: 10)
- `max_log_size` — per-log file size cap in bytes (default: 2097152 = 2MB, 0 to disable)
- `default_routine` — fallback routine name (default: "develop")
- `hooks.*` — lifecycle hook routine names (see migration 10)

## Message ID Format

A message ID has the form `<chain>-<seq>`:

- **Chain ID**: `YYYYMMDDHHmmss` + 2-digit counter (e.g. `2025022514320000`).
  Generated once for the root message; all messages in a chain share it.
- **Sequence number**: starts at `0`, increments by 1 for each follow-up.

Commands accept full IDs, chain IDs, or unique prefixes.

## Full CLI

```
decree                                # Smart default (process)
decree init                           # Initialize project
decree process                        # Process all migrations + drain inbox
decree starter [NAME]                 # Build starter prompt, copy or launch AI
decree routine                        # List routines (interactive)
decree routine <name>                 # Show routine detail + run pre-checks
decree verify                         # Run all routine pre-checks
decree daemon [--interval SECS]       # Daemon: monitor inbox + cron
decree status                         # Show progress
decree log [ID]                       # Show execution log
decree help                           # Verbose help (see migration 12)
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

Summarizes progress: processed migrations, pending inbox messages,
recent message history.

## `decree log`

Reads log files from message directories in `.decree/runs/`.
Without an ID: shows the most recent message's log. With a chain ID:
shows all logs in that chain. If multiple attempts exist (`routine.log`,
`routine-2.log`, etc.), all are shown with attempt headers.

## Acceptance Criteria

- [ ] `decree init` creates `.decree/` with subdirs: `routines/`, `starters/`,
      `cron/`, `inbox/` (with `done/` and `dead/`), `runs/`
- [ ] `migrations/` is created at project root with empty `processed.md`
- [ ] AI backend is auto-detected from opencode/claude/copilot
- [ ] If no AI backend found, suggests opencode.ai and sets `opencode` as default
- [ ] Git is detected via `which git` and `git rev-parse --is-inside-work-tree`
- [ ] If git detected, user is offered git stash hooks (default Yes)
- [ ] If git hooks accepted, hook routines are created and config populated
- [ ] If git hooks declined or git not found, hooks are empty strings
- [ ] `config.yml` is written with detected AI command and hook values
- [ ] All CLI subcommands are accepted and dispatched correctly
- [ ] `decree status` shows processed migrations, pending inbox, recent history
- [ ] `decree log` displays routine output for specified message
- [ ] `decree log` without ID shows most recent message's log
- [ ] Bare `decree` dispatches to `decree process`
