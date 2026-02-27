---
routine: rust-develop
---

# 06: Routine System

## Overview

Routines are the executable units that process messages. They live in
`.decree/routines/` in two supported formats: shell scripts (`.sh`) and
Jupyter notebooks (`.ipynb`). This spec covers routine formats, description
extraction, parameter injection, discovery precedence, the selection chain,
custom variables, and how routines spawn follow-up work.

## Requirements

### Routine Formats

#### Shell Scripts (`.sh`)

A bash script with a description comment block and parameter declarations:

```bash
#!/usr/bin/env bash
# Routine Name
#
# Description of what this routine does. The router reads this
# comment block to build the selection prompt.
set -euo pipefail

# Parameters (decree injects these as env vars)
spec_file="${spec_file:-}"
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"

# ... routine logic
```

**Description extraction**: starting from line 1 (or line 2 if line 1 is a
shebang), collect contiguous lines beginning with `#`. Strip the leading
`# ` (or lone `#` for blank comment lines) to produce description text.

**Parameter injection**: standard and custom parameters are set as
**environment variables** before execution. The `${var:-default}` pattern
lets scripts run standalone while decree overrides values at execution time.

#### Notebooks (`.ipynb`)

A Jupyter notebook with Python kernel:
- A parameters cell tagged `["parameters"]` for papermill injection
- `%%bash` cells for actual work

```python
spec_file = ""          # Path to the input file (spec or task; empty if inline)
message_file = ""        # Path to the message file (<msg-dir>/message.md)
message_id = ""          # Full message ID (chain-seq)
message_dir = ""         # This message's run directory
chain = ""               # Chain ID
seq = ""                 # Sequence number in chain
```

**Description extraction**: the first markdown cell content.

**Parameter injection**: values are passed as `-p` flags to papermill.

### Standard Parameters

Both formats receive the same standard parameters:

| Parameter      | Description                                      |
|----------------|--------------------------------------------------|
| `spec_file`    | Path to the spec file (empty for task messages)  |
| `message_file` | Path to `message.md` in the run directory        |
| `message_id`   | Full message ID (`<chain>-<seq>`)                |
| `message_dir`  | Run directory path                               |
| `chain`        | Chain ID                                         |
| `seq`          | Sequence number                                  |

All parameters are file paths or identifiers — the routine reads file
contents itself. This avoids shell-escaping issues with large content.

### Routine Discovery

The routine name from the message maps to a file in `.decree/routines/`.
Precedence depends on the `notebook_support` config flag:

**When `notebook_support: true`:**
1. Check for `.decree/routines/<name>.ipynb`
2. Check for `.decree/routines/<name>.sh`

Notebooks take precedence — the user opted into the richer format.

**When `notebook_support: false` (default):**
1. Check for `.decree/routines/<name>.sh`
2. `.ipynb` files are **ignored** entirely — not checked, not listed

If the routine name has an explicit extension (e.g. `routine: develop.sh`
or `routine: develop.ipynb` in message frontmatter), the extension is
used directly and precedence rules are skipped.

If neither exists (or only `.ipynb` exists with notebooks disabled),
return `RoutineNotFound`.

When listing available routines (for the router prompt and interactive
`decree run`), scan `.decree/routines/` for `*.sh` and (when notebook
support is enabled) `*.ipynb`. Deduplicate by stem — if both exist,
list the name once with the preferred format noted.

### Routine Selection

For each message, the routine is determined during normalization by:
1. **Message frontmatter**: `routine:` field in the inbox message (if present)
2. **Spec frontmatter**: `routine:` field in the spec's YAML (for spec messages)
3. **Router AI**: `commands.router` picks from available routines based on
   the message body (see spec 05 for normalization details)
4. **Config default**: `default_routine` from config.yml
5. **Fallback**: "develop" if nothing else specified

The routine name is resolved to a file via the discovery rules above.

### Custom Routine Variables

Routines can declare extra parameters beyond the standard set (`spec_file`,
`message_file`, `message_id`, `message_dir`, `chain`, `seq`). The
processor discovers these at runtime by reading the routine file.

**Discovery by format**:

- **Shell scripts**: the processor reads the script and collects variable
  names from lines matching `^[a-z_][a-z0-9_]*=` (simple assignment at
  start of line), stopping at the first line that is not a comment, blank,
  shebang, `set` builtin, or assignment. Standard parameter names are
  excluded — the remainder are custom parameters.
- **Notebooks**: the processor reads the parameters cell (tagged
  `["parameters"]`) and parses Python variable assignments. Standard
  parameter names are excluded.

Example — a shell script with a `target_branch` parameter:

```bash
#!/usr/bin/env bash
# PR Review routine
set -euo pipefail

spec_file="${spec_file:-}"
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
target_branch="${target_branch:-main}"   # Custom parameter
```

Example — a notebook with the same custom parameter:

```python
spec_file = ""
message_file = ""
message_id = ""
message_dir = ""
chain = ""
seq = ""
target_branch = ""       # Custom: branch to target for PRs
```

Custom variables are populated from inbox message frontmatter. If a message
includes a field matching a custom parameter name, that value is injected:

```yaml
---
id: 2025022514320000-0
chain: 2025022514320000
seq: 0
type: task
routine: pr-review
target_branch: main
---
Review the changes on the current branch.
```

Behaviour:
- At execution time, the processor reads the routine to discover declared
  parameter names
- For each parameter beyond the standard set, it checks the message
  frontmatter for a matching field
- **Shell scripts**: matching fields are set as environment variables
- **Notebooks**: matching fields are passed as additional `-p` flags to
  papermill
- Unrecognized frontmatter fields (not matching any parameter) are ignored
- Missing values use the routine's default (typically empty string)
- The router AI is **not** used to fill custom variables — only the user
  fills them (interactively or via frontmatter)

### Routines Writing to Inbox

Routines can spawn follow-up work by writing new message files to
`.decree/inbox/`. The only requirement is the filename convention —
frontmatter is optional because the processor normalizes everything.

Minimal example (just a filename and body — works in both `.sh` routines
and notebook `%%bash` cells):

```bash
NEXT_SEQ=$((seq + 1))
cat > .decree/inbox/${chain}-${NEXT_SEQ}.md << EOF
Fix type errors in src/auth.rs that were introduced.
EOF
```

The processor derives `chain` and `seq` from the filename, sets
`type: task`, and uses the router to pick the routine. Routines that know
the desired routine can include partial frontmatter:

```bash
NEXT_SEQ=$((seq + 1))
cat > .decree/inbox/${chain}-${NEXT_SEQ}.md << EOF
---
routine: develop
---
Fix type errors in src/auth.rs that were introduced.
EOF
```

After the current routine completes, the processing loop checks the inbox
for messages with the same chain ID and processes them depth-first.

### Python Venv

Only relevant when `notebook_support: true`. `ensure_venv()` is called
before notebook routine execution. Shell script routines never require a
venv. When `notebook_support: false` (the default), no venv is created
and `.ipynb` routines are not executed. Creates `.decree/venv/` with
papermill + ipykernel if missing. Respects `DECREE_VENV` env var.

## Acceptance Criteria

### Custom Routine Variables

- **Given** a notebook routine's parameters cell declares `target_branch = ""`
  beyond the standard parameters
  **When** the processor reads the routine before execution
  **Then** `target_branch` is recognized as a custom variable

- **Given** a shell script routine declares `target_branch="${target_branch:-}"`
  beyond the standard parameters
  **When** the processor reads the routine before execution
  **Then** `target_branch` is recognized as a custom variable

- **Given** an inbox message has `target_branch: main` in its frontmatter
  **When** the matching notebook routine declares `target_branch` as a parameter
  **Then** `-p target_branch "main"` is added to the papermill invocation

- **Given** an inbox message has `target_branch: main` in its frontmatter
  **When** the matching shell script routine declares `target_branch` as a parameter
  **Then** the environment variable `target_branch=main` is set during execution

- **Given** an inbox message has a frontmatter field `unknown_field: value`
  **When** the routine does not declare `unknown_field` as a parameter
  **Then** the field is ignored (no injection occurs)

- **Given** a routine declares a custom parameter `target_branch`
  **When** the inbox message does not include `target_branch` in frontmatter
  **Then** the routine's default value is used

### Routine Discovery and Format Dispatch

- **Given** `.decree/routines/develop.sh` exists
  **When** a message with `routine: develop` is processed
  **Then** the shell script is executed via `bash` (not papermill)

- **Given** `notebook_support: true` and `.decree/routines/develop.ipynb`
  exists but `develop.sh` does not
  **When** a message with `routine: develop` is processed
  **Then** the notebook is executed via papermill

- **Given** `notebook_support: true` and both `develop.sh` and
  `develop.ipynb` exist
  **When** the processor resolves the routine
  **Then** `develop.ipynb` is selected (notebooks take precedence when
  notebook support is enabled)

- **Given** `notebook_support: false` and both `develop.sh` and
  `develop.ipynb` exist
  **When** the processor resolves the routine
  **Then** `develop.sh` is selected (`.ipynb` files are ignored)

- **Given** `notebook_support: false` and only `develop.ipynb` exists
  **When** the processor resolves the routine
  **Then** `RoutineNotFound` error is returned

- **Given** neither `develop.sh` nor `develop.ipynb` exists
  **When** the processor resolves the routine
  **Then** `RoutineNotFound` error is returned

- **Given** a message specifies `routine: develop.sh` (explicit extension)
  **When** the processor resolves the routine
  **Then** `develop.sh` is used directly, skipping precedence rules

- **Given** a shell script routine runs successfully
  **When** the run completes
  **Then** `<msg-dir>/routine.log` contains the combined stdout/stderr
  **And** no `output.ipynb` or `papermill.log` is created

- **Given** a shell script routine has a description comment block
  **When** the router builds its selection prompt
  **Then** the description is extracted and included alongside notebook
  descriptions (when notebook support is enabled)

- **Given** `decree run` is invoked interactively
  **When** routines are listed for selection
  **Then** `.sh` routines appear, and `.ipynb` routines also appear when
  notebook support is enabled, deduplicated by stem

### Venv

- **Given** `notebook_support: true` and `.decree/venv/` does not exist
  **When** a notebook routine is about to execute
  **Then** `ensure_venv()` creates the venv with papermill and ipykernel

- **Given** a shell script routine is about to execute
  **When** `.decree/venv/` does not exist
  **Then** `ensure_venv()` is not called and execution proceeds without a venv

- **Given** `notebook_support: false` (default)
  **When** a `.ipynb` file exists in `.decree/routines/`
  **Then** it is ignored by discovery and never executed
