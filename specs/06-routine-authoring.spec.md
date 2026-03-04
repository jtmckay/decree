# 06: Routine Authoring Documentation

## Overview

Decree generates a `ROUTINES.md` document at init time explaining how to
author custom routines. Emphasis on parameter mapping, custom parameter
discovery, pre-check sections, and the AI-specific nature of routines.

## Document Sections

### 1. Overview

A routine is a shell script in `.decree/routines/` that decree executes
with env vars populated from message frontmatter and runtime context.
Routines are AI-specific — they invoke AI tools to perform work. They
can be nested in subdirectories for organization.

### 2. Standard Parameter Mapping Table

| Frontmatter field | Env var in routine | Notes |
|---|---|---|
| `input_file` | `input_file` | Optional. Path to input file |
| *(auto)* | `message_file` | Path to message.md in run directory |
| *(auto)* | `message_id` | Unique message identifier |
| *(auto)* | `message_dir` | Run directory path |
| `chain` | `chain` | Chain ID for multi-step workflows |
| `seq` | `seq` | Sequence number in chain |

### 3. Custom Parameter Discovery

Decree scans the routine from top to bottom:
1. Skips: shebang, comments, blanks, `set` builtins, pre-check block
2. Matches: `var_name="${var_name:-default_value}"`
3. Stops at first non-matching line
4. Excludes standard parameter names
5. Remaining variables are custom parameters
6. Empty defaults (`${var:-}`) mean optional with no default

Custom values come from message frontmatter — any key not in the
standard set is passed as an env var.

### 4. Pre-Check Section

Every routine must include a pre-check gate:

```bash
# Pre-check: verify dependencies
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then
    command -v opencode >/dev/null 2>&1 || { echo "opencode not found"; exit 1; }
    exit 0
fi
```

Rules:
- Gate on `DECREE_PRE_CHECK=true` env var
- Place after standard params, before custom params
- Exit 0 = routine is ready, exit non-zero = not ready
- Print missing dependency to stderr on failure
- Used by `decree routine <name>` and `decree verify`

### 5. Minimal Routine Example

```bash
#!/usr/bin/env bash
# My Routine
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
# my_option     - Optional. Custom option from frontmatter
# another_flag  - Optional. Custom flag from frontmatter
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
input_file="${input_file:-}"

# Pre-check
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then
    command -v opencode >/dev/null 2>&1 || { echo "opencode not found"; exit 1; }
    exit 0
fi

# Custom params
my_option="${my_option:-default_value}"
another_flag="${another_flag:-false}"

# --- Implementation ---
if [ -n "$input_file" ] && [ -f "$input_file" ]; then
    WORK_FILE="$input_file"
else
    WORK_FILE="$message_file"
fi

opencode run "Read ${WORK_FILE} and implement. Option: ${my_option}"
```

### 6. Corresponding Message Frontmatter

```yaml
---
input_file: ./path/to/source.txt
routine: my-routine
my_option: custom_value
another_flag: true
---
Any additional message body here.
```

Key points:
- `input_file` in frontmatter maps to `input_file` env var in script
- `routine` selects by filename without `.sh` (or path for nested)
- Any non-standard key becomes a custom field env var

### 7. Comment Header Format

```bash
#!/usr/bin/env bash
# Title Here
#
# Short description (used in `decree routine` list).
# Additional lines shown in detail view.
```

### 8. Nested Routines

Routines can be organized in subdirectories:

```
.decree/routines/
├── develop.sh           # routine: develop
├── deploy/
│   ├── staging.sh       # routine: deploy/staging
│   └── production.sh    # routine: deploy/production
└── review/
    └── pr.sh            # routine: review/pr
```

### 9. Tips and Gotchas

- **Pre-check required**: Every routine must have a pre-check section
- **Parameter comments**: Use `# --- Parameters ---` block to document vars
- **Optional marker**: Mark optional params with "Optional." in the comment
- **Discovery boundary**: Use a comment like `# --- Implementation ---`
- **Default values**: Use meaningful defaults where possible
- **`set -euo pipefail`**: Always include — decree expects non-zero on failure
- **AI-specific**: Routines should invoke an AI tool — they are not
  general-purpose shell scripts

## File Location

`ROUTINES.md` is generated in the project root during `decree init`.

## Acceptance Criteria

- [ ] `ROUTINES.md` is generated during `decree init`
- [ ] Standard parameter mapping table is included
- [ ] Custom parameter discovery algorithm is documented
- [ ] Pre-check section is documented with example
- [ ] Minimal routine example shows standard params, pre-check, custom params
- [ ] Frontmatter example shows how to invoke routine and pass custom fields
- [ ] Comment header format for description extraction is documented
- [ ] Nested routine directory structure is documented
- [ ] Tips section documents parameter comment block convention
