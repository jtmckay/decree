# 04: Routine System

## Overview

Routines are shell scripts in `.decree/routines/` that decree executes
with env vars populated from message frontmatter and runtime context.
Routines are AI-specific — each targets a particular AI tool. They can
be nested in subdirectories for organization.

## Shell Script Format

```bash
#!/usr/bin/env bash
# Routine Name
#
# Description of what this routine does.
# This text is shown by `decree routine`.
set -euo pipefail

# --- Parameters ---
# message_file  - Path to message.md in the run directory
# message_id    - Full message ID (e.g., D0001-1432-01-add-auth-0)
# message_dir   - Run directory path
# chain         - Chain ID (D<NNNN>-HHmm-<name>)
# seq           - Sequence number
# my_option     - Optional. Custom parameter from frontmatter
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"

# Pre-check gate
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then
    command -v {AI_CMD} >/dev/null 2>&1 || { echo "{AI_CMD} not found" >&2; exit 1; }
    exit 0
fi

# Custom parameters (discovered from frontmatter)
my_option="${my_option:-default_value}"

# --- Implementation starts here ---
```

## Standard Parameters

| Parameter      | Description                                      |
|----------------|--------------------------------------------------|
| `message_file` | Path to `message.md` in the run directory        |
| `message_id`   | Full message ID (e.g., `D0001-1432-01-add-auth-0`) |
| `message_dir`  | Run directory path                               |
| `chain`        | Chain ID                                         |
| `seq`          | Sequence number                                  |

## Parameter Comment Block

Routines should declare their parameters in a `# --- Parameters ---`
comment block between `set -euo pipefail` and the variable assignments.
Each line has the format:

```
# var_name      - Description
# var_name      - Optional. Description
```

The CLI uses "Optional" to determine which parameters are required vs
optional. Parameters without "Optional" are always provided by decree.

## Parameter Injection

Parameters are set as **environment variables** before execution. The
`${var:-default}` pattern lets scripts run standalone while decree
overrides values at execution time.

## Pre-Check Section

Every routine **must** define a pre-check section. When the environment
variable `DECREE_PRE_CHECK` is set to `"true"`, the routine runs its
pre-check logic and exits:

- Exit 0: routine is ready to run (all dependencies met)
- Exit non-zero: routine cannot run (print what's missing to stderr)

Pre-checks should verify:
- Required external tools are installed (AI CLI, cargo, etc.)
- Required files or configuration exist
- Any other prerequisites

The pre-check gate must appear after standard parameter declarations
but before any custom parameters or implementation logic.

## Routine Discovery

Routines are discovered by scanning `.decree/routines/` recursively:

- All `*.sh` files are discovered
- Nested directories are supported: `.decree/routines/deploy/staging.sh`
  becomes routine `deploy/staging`
- Routine names are derived from the path relative to `.decree/routines/`
  without the `.sh` extension

### Description Extraction

1. Skip the shebang line (`#!/...`)
2. Next comment line is the routine title (e.g. `# Transcribe`)
3. Skip blank comment lines (`#`)
4. Collect subsequent comment lines as description, stripping `# `
5. First line of that block is the short description
6. Stop at first non-comment line

### Custom Parameter Discovery

Decree scans the script top-to-bottom:

1. Skip shebang, comment lines, blank lines, `set` builtins
2. Skip the `DECREE_PRE_CHECK` block (from `if` to `fi`)
3. Match assignments of the form `var="${var:-default}"`
4. Stop at the first non-matching line
5. Exclude standard parameter names from results
6. Remaining matches are custom parameters with defaults from `:-default`

## Routine Selection Chain

For each message, the routine is determined by:
1. Message frontmatter `routine:` field
2. Router AI selects from available routines
3. Config `default_routine`
4. Fallback: "develop"

## Routines Writing to Inbox

Routines can spawn follow-up work by writing files to `.decree/outbox/`:

```bash
cat > ".decree/outbox/fix-type-errors.md" << EOF
Fix type errors introduced by the implementation.
EOF
```

Outbox files can be plain markdown or include optional YAML frontmatter
(e.g., `routine:` to target a specific routine, or custom fields).
Chain, seq, and id are always assigned by decree — any values in
frontmatter for those fields are ignored.

After the routine completes, decree collects all `*.md` files from the
outbox (sorted alphabetically), assigns each one the current `chain`
and incrementing `seq`, moves them to the inbox as properly formatted
messages, and clears the outbox. Non-`.md` files in the outbox trigger
a warning. If a follow-up's `seq` would exceed `max_depth`, the file
is moved to `.decree/outbox/dead/` instead. Follow-up messages are
then processed depth-first within the chain.

## Acceptance Criteria

- [ ] Shell scripts in `.decree/routines/` are discovered recursively
- [ ] Nested directory routines use path-based names (e.g. `deploy/staging`)
- [ ] Description extraction follows the comment header convention
- [ ] Standard parameters are injected as env vars
- [ ] Custom parameters are discovered from `var="${var:-default}"` pattern
- [ ] Custom parameter values come from message frontmatter
- [ ] Pre-check gate exits early when `DECREE_PRE_CHECK=true`
- [ ] Pre-check returns 0 when dependencies are met, non-zero otherwise
- [ ] Routine selection follows the 4-step chain
- [ ] Routines can spawn follow-up messages by writing to `.decree/outbox/`
- [ ] Decree collects only `*.md` outbox files, warns on non-`.md` files
- [ ] Outbox files exceeding `max_depth` are moved to `.decree/outbox/dead/`
- [ ] Outbox is cleared after collection
- [ ] Pre-check failures print to stderr (not stdout)
