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
# message_id    - Full message ID (<chain>-<seq>)
# message_dir   - Run directory path
# chain         - Chain ID
# seq           - Sequence number
# input_file    - Optional. Path to input file (e.g., migration file)
# my_option     - Optional. Custom parameter from frontmatter
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
input_file="${input_file:-}"

# Pre-check gate
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then
    command -v opencode >/dev/null 2>&1 || { echo "opencode not found"; exit 1; }
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
| `message_id`   | Full message ID (`<chain>-<seq>`)                |
| `message_dir`  | Run directory path                               |
| `chain`        | Chain ID                                         |
| `seq`          | Sequence number                                  |
| `input_file`   | Optional. Path to input file (from frontmatter)  |

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
2. Migration frontmatter `routine:` field (for spec messages)
3. Router AI selects from available routines
4. Config `default_routine`
5. Fallback: "develop"

## Routines Writing to Inbox

Routines can spawn follow-up work by writing to `.decree/inbox/`:

```bash
NEXT_SEQ=$((seq + 1))
cat > .decree/inbox/${chain}-${NEXT_SEQ}.md << EOF
Fix type errors introduced by the implementation.
EOF
```

After the current routine completes, the processing loop checks the
inbox for messages with the same chain ID and processes them depth-first.

## Acceptance Criteria

- [ ] Shell scripts in `.decree/routines/` are discovered recursively
- [ ] Nested directory routines use path-based names (e.g. `deploy/staging`)
- [ ] Description extraction follows the comment header convention
- [ ] Standard parameters are injected as env vars
- [ ] `input_file` frontmatter maps to `input_file` env var
- [ ] Custom parameters are discovered from `var="${var:-default}"` pattern
- [ ] Custom parameter values come from message frontmatter
- [ ] Pre-check gate exits early when `DECREE_PRE_CHECK=true`
- [ ] Pre-check returns 0 when dependencies are met, non-zero otherwise
- [ ] Routine selection follows the 5-step chain
- [ ] Routines can spawn follow-up messages by writing to inbox
