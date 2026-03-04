# Authoring Routines

A routine is a shell script in `.decree/routines/` that decree executes
with environment variables populated from message frontmatter and runtime
context. Routines are AI-specific — they invoke AI tools to perform work.
They can be nested in subdirectories for organization.

## Standard Parameter Mapping

Every routine receives these environment variables automatically:

| Frontmatter field | Env var in routine | Notes |
|---|---|---|
| `input_file` | `input_file` | Optional. Path to input file |
| *(auto)* | `message_file` | Path to message.md in run directory |
| *(auto)* | `message_id` | Unique message identifier |
| *(auto)* | `message_dir` | Run directory path |
| `chain` | `chain` | Chain ID for multi-step workflows |
| `seq` | `seq` | Sequence number in chain |

Variables marked *(auto)* are injected by decree at runtime and do not
come from frontmatter.

## Custom Parameter Discovery

Decree scans the routine from top to bottom to discover custom parameters:

1. **Skips**: shebang (`#!`), comments (`#`), blank lines, `set` builtins,
   and the pre-check block
2. **Matches**: lines of the form `var_name="${var_name:-default_value}"`
3. **Stops**: at the first non-matching line after the custom params section
4. **Excludes**: standard parameter names listed above
5. **Remaining**: variables are custom parameters
6. **Empty defaults**: `${var:-}` means optional with no default

Custom values come from message frontmatter — any key not in the standard
set is passed as an environment variable to the routine.

## Pre-Check Section

Every routine must include a pre-check gate. Decree uses this to verify
that all dependencies are available before running the routine.

```bash
# Pre-check: verify dependencies
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then
    command -v opencode >/dev/null 2>&1 || { echo "opencode not found"; exit 1; }
    exit 0
fi
```

Rules:

- Gate on the `DECREE_PRE_CHECK=true` environment variable
- Place after standard params, before custom params
- `exit 0` means the routine is ready; non-zero means not ready
- Print the missing dependency name to stderr on failure
- Used by `decree routine <name>` and `decree verify`

## Minimal Routine Example

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

## Corresponding Message Frontmatter

To invoke a routine and pass custom fields, use YAML frontmatter in
the message file:

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

- `input_file` in frontmatter maps to the `input_file` env var in the script
- `routine` selects by filename without `.sh` (or path for nested routines)
- Any non-standard key becomes a custom field environment variable

## Comment Header Format

Decree extracts routine titles and descriptions from the comment header:

```bash
#!/usr/bin/env bash
# Title Here
#
# Short description (used in `decree routine` list).
# Additional lines shown in detail view.
```

The first comment line after the shebang is the title. A blank comment
line (`#`) separates the title from the description body.

## Nested Routines

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

Reference nested routines by their path relative to `.decree/routines/`
without the `.sh` extension.

## Tips and Gotchas

- **Pre-check required**: Every routine must have a pre-check section
- **Parameter comments**: Use a `# --- Parameters ---` block to document
  all variables the routine expects
- **Optional marker**: Mark optional params with "Optional." in the comment
- **Discovery boundary**: Use a comment like `# --- Implementation ---`
  to clearly separate params from logic
- **Default values**: Use meaningful defaults where possible
- **`set -euo pipefail`**: Always include — decree expects non-zero on failure
- **AI-specific**: Routines should invoke an AI tool — they are not
  general-purpose shell scripts
