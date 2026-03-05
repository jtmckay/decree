# 06: Routine Authoring Documentation

## Overview

Routine authoring documentation is available in two places:
1. **`decree help`** — includes a full "Defining Routines" section
2. **`.decree/prompts/routine.md`** — a prompt template that primes an
   AI assistant to help build new routines

There is no standalone `ROUTINES.md` file in the project root. The
documentation lives where it's most useful: in the help system for
human reference, and as a prompt template for AI-assisted authoring.

## Content

Both locations contain the same core documentation covering:

### 1. Overview

A routine is a shell script in `.decree/routines/` that decree executes
with env vars populated from message frontmatter and runtime context.
Routines are AI-specific — they invoke AI tools to perform work. They
can be nested in subdirectories for organization.

### 2. Standard Parameter Mapping Table

| Frontmatter field | Env var in routine | Notes |
|---|---|---|
| *(auto)* | `message_file` | Path to message.md in run directory |
| *(auto)* | `message_id` | Unique message identifier |
| *(auto)* | `message_dir` | Run directory path |
| `chain` | `chain` | Chain ID (`D<NNNN>-HHmm-<name>`) |
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
    command -v {AI_CMD} >/dev/null 2>&1 || { echo "{AI_CMD} not found" >&2; exit 1; }
    exit 0
fi
```

Rules:
- Gate on `DECREE_PRE_CHECK=true` env var
- Place after standard params, before custom params
- Exit 0 = routine is ready, exit non-zero = not ready
- Print missing dependency to **stderr** on failure
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
# message_id    - Full message ID (e.g., D0001-1432-01-add-auth-0)
# message_dir   - Run directory path (contains logs from prior attempts)
# chain         - Chain ID
# seq           - Sequence number
# my_option     - Optional. Custom option from frontmatter
# another_flag  - Optional. Custom flag from frontmatter
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"

# Pre-check
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then
    command -v {AI_CMD} >/dev/null 2>&1 || { echo "{AI_CMD} not found" >&2; exit 1; }
    exit 0
fi

# Custom params
my_option="${my_option:-default_value}"
another_flag="${another_flag:-false}"

# --- Implementation ---
{AI_CMD} run "Read ${message_file} and implement. Option: ${my_option}
Previous attempt logs (if any) are in ${message_dir} for context."
```

### 6. Corresponding Message Frontmatter

```yaml
---
routine: my-routine
my_option: custom_value
another_flag: true
---
Any additional message body here.
```

Key points:
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
- **Run directory**: Use `${message_dir}` for context from prior attempts
- **AI-specific**: Routines should invoke an AI tool — they are not
  general-purpose shell scripts
- **{AI_CMD}**: Use the `{AI_CMD}` placeholder in templates; replaced at
  init time with the configured `commands.ai_command`

## Acceptance Criteria

- [ ] Routine authoring docs are included in `decree help` output
- [ ] `.decree/prompts/routine.md` is generated during `decree init`
- [ ] Prompt template can be used via `decree prompt routine` to prime AI
- [ ] No standalone `ROUTINES.md` is created in project root
- [ ] Standard parameter mapping table is documented
- [ ] Custom parameter discovery algorithm is documented
- [ ] Pre-check section is documented with example (stderr output)
- [ ] Minimal routine example shows standard params, pre-check, custom params
- [ ] Example references `${message_dir}` for prior attempt context
- [ ] Frontmatter example shows how to invoke routine and pass custom fields
- [ ] Comment header format for description extraction is documented
- [ ] Nested routine directory structure is documented
- [ ] Tips section documents `{AI_CMD}` placeholder usage
