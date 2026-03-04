# 02: Default Templates

## Overview

All files `decree init` writes into a new project. Starter templates teach
the AI how to write migrations. Routine templates define default execution
workflows. Source files live in `src/templates/` and are embedded at compile
time via `include_str!()`. Routine templates use `{AI_CMD}` placeholder
replaced at init time based on the detected AI backend.

## Template Embedding

Templates embedded from `src/templates/` at compile time:
- `spec.md` — migration/spec template (starter)
- `develop.sh` — default routine
- `rust-develop.sh` — Rust-specific routine
- `gitignore` — .decree/.gitignore

## `.decree/starters/spec.md` — Spec/Migration Template

```markdown
# Spec Template

Each migration is a self-contained unit of work. Migrations are immutable —
once created, they are processed exactly once and never modified.

## Format

    ---
    routine: develop
    ---
    # NN: Title

    ## Overview
    Brief description of what this migration accomplishes.

    ## Requirements
    Detailed technical requirements.

    ## Acceptance Criteria
    - [ ] Criterion 1
    - [ ] Criterion 2

## Rules

- **Naming**: `NN-descriptive-name.md` (e.g., `01-add-auth.md`)
- **Frontmatter**: Optional YAML with `routine:` field (defaults to develop)
- **Ordering**: Alphabetical by filename determines execution order
- **Immutability**: Never edit a processed migration — create a new one
- **Self-contained**: Each migration should be independently implementable
```

## `.decree/routines/develop.sh`

```bash
#!/usr/bin/env bash
# Develop
#
# Default routine that delegates work to an AI assistant.
# Reads the input file or task message, prompts the AI to implement all
# requirements, then verifies acceptance criteria are met.
set -euo pipefail

# --- Parameters ---
# message_file  - Path to message.md in the run directory
# message_id    - Full message ID (<chain>-<seq>)
# message_dir   - Run directory path
# chain         - Chain ID
# seq           - Sequence number
# input_file    - Optional. Path to input file (e.g., migration file)
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
input_file="${input_file:-}"

# Pre-check: verify AI tool is available
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then
    command -v opencode >/dev/null 2>&1 || { echo "opencode not found"; exit 1; }
    exit 0
fi

# Determine the work description
if [ -n "$input_file" ] && [ -f "$input_file" ]; then
    WORK_FILE="$input_file"
else
    WORK_FILE="$message_file"
fi

# Implementation
opencode run "Read the work description at ${WORK_FILE} and implement all
requirements. Follow best practices: clean code, proper error handling,
and tests where appropriate."

# Verification
opencode run "Read the work description at ${WORK_FILE}. Verify that all
requirements and acceptance criteria are met. Run any tests. Report what
passes and what fails. Exit 0 if everything passes, exit 1 if anything fails."
```

## `.decree/routines/rust-develop.sh`

```bash
#!/usr/bin/env bash
# Rust Develop
#
# Rust-specific development routine. Delegates implementation to AI,
# builds and tests, then hands failures to AI for fix-up.
set -euo pipefail

# --- Parameters ---
# message_file  - Path to message.md in the run directory
# message_id    - Full message ID (<chain>-<seq>)
# message_dir   - Run directory path
# chain         - Chain ID
# seq           - Sequence number
# input_file    - Optional. Path to input file (e.g., migration file)
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
input_file="${input_file:-}"

# Pre-check: verify AI tool and cargo are available
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then
    command -v opencode >/dev/null 2>&1 || { echo "opencode not found"; exit 1; }
    command -v cargo >/dev/null 2>&1 || { echo "cargo not found"; exit 1; }
    exit 0
fi

if [ -n "$input_file" ] && [ -f "$input_file" ]; then
    WORK_FILE="$input_file"
else
    WORK_FILE="$message_file"
fi

# Step 1: Implementation
opencode run "You are a senior Rust engineer. Read ${WORK_FILE} and implement
all requirements with proper error handling and tests."

# Step 2: Build and test
echo "=== Building (release) ==="
cargo build --release 2>&1 | tee "${message_dir}/build.log" || true
echo "=== Running tests ==="
cargo test 2>&1 | tee "${message_dir}/test-output.log" || true

# Step 3: QA
opencode run "Read ${WORK_FILE}, build output at ${message_dir}/build.log,
test output at ${message_dir}/test-output.log. Fix any failures. Run cargo
build --release and cargo test again. Exit 0 only if everything passes."
```

## `.decree/.gitignore`

```
inbox/
runs/
```

## Acceptance Criteria

- [ ] `decree init` creates `develop.sh` and `rust-develop.sh` in `.decree/routines/`
- [ ] Both routines use `{AI_CMD}` placeholder replaced with detected AI command
- [ ] Both routines include a pre-check section gated on `DECREE_PRE_CHECK`
- [ ] `.decree/starters/spec.md` contains the migration template
- [ ] `.decree/.gitignore` excludes `inbox/` and `runs/`
- [ ] Template source files exist in `src/templates/` and are embedded via `include_str!()`
- [ ] Routine templates have description comment headers for `decree routine` extraction
