---
routine: develop
---

# 02: Default Templates

## Overview

All files that `decree init` writes into a new project. Plan templates
teach the AI how to write SOWs and specs. Routine templates define the
default execution workflows. Template source files live in `src/templates/`
and are embedded at compile time via `include_str!()`. Routine templates
use `{AI_CMD}` and `{ALLOWED_TOOLS}` placeholders replaced at init time
based on the selected AI provider. Notebook templates are only written
when notebook support is enabled.

## Requirements

### Template Embedding

`decree init` writes these files with the exact content shown below. They
are the prompts and routines that shape how decree plans and executes work.
The content is embedded from template files in `src/templates/` at compile
time via `include_str!()`. Template files: `src/templates/sow.md`,
`src/templates/spec.md`, `src/templates/gitignore`, `src/templates/develop.sh`,
`src/templates/develop.ipynb`, `src/templates/rust-develop.sh`,
`src/templates/rust-develop.ipynb`. All routine templates use `{AI_CMD}` and
`{ALLOWED_TOOLS}` placeholders which are replaced at init time based on the
selected AI provider. Notebook templates are only written when notebook
support is enabled.

### `.decree/plans/sow.md` — SOW Template

This file is injected into the planning prompt (spec 09) when the user
runs `decree plan`. It teaches the AI how to write a good SOW.

```markdown
# Statement of Work Template

A Statement of Work (SOW) captures the business intent behind a body of work.
It defines the value being delivered, the problems being solved, and the outcomes
expected — written from the perspective of the people who benefit from the work.

## Structure

A SOW should include:

- **Title**: Clear, outcome-oriented project name
- **Business Context**: Why this work matters — the problem, opportunity, or need
- **Jobs to Be Done**: What users/stakeholders need to accomplish (framed as jobs)
- **User Scenarios**: Concrete narratives showing how people interact with the solution
- **Scope**: Boundaries — what is and isn't included in this engagement
- **Deliverables**: Tangible outputs that fulfill the jobs to be done
- **Acceptance Criteria**: How we know the work is complete and successful
- **Assumptions & Constraints**: Known limitations, dependencies, or preconditions

## Writing Guidelines

- Lead with **why** before **what** — business value before technical detail
- Frame work as **jobs to be done**: "When [situation], I want to [motivation], so I can [outcome]"
- Use **user scenarios** to ground abstract requirements in real usage
- Keep scope boundaries explicit — what's excluded is as important as what's included
- Deliverables should map back to jobs and scenarios, not implementation artifacts
- Acceptance criteria should be verifiable from a user/stakeholder perspective

## Example

# SOW: Secure Account Access

## Business Context

Users currently have no way to maintain persistent sessions across visits.
Every interaction requires re-identification, creating friction and abandonment.
Providing secure account access increases retention and enables personalized
experiences that drive engagement.

## Jobs to Be Done

1. When I return to the application, I want to resume where I left off,
   so I don't lose progress or repeat steps.
2. When I create an account, I want confidence my credentials are secure,
   so I can trust the platform with my information.
3. When I'm done using the application, I want to end my session cleanly,
   so others on shared devices can't access my account.

## User Scenarios

- **New visitor signup**: A first-time user provides an email and password,
  receives confirmation, and lands in their personalized workspace.
- **Returning user login**: A registered user enters credentials and is
  returned to their previous state within seconds.
- **Shared device logout**: A user on a library computer logs out and
  verifies the next person sees no trace of their session.

## Scope

**In scope:**

- Account creation and credential management
- Session-based login and logout
- Secure credential storage

**Out of scope (future work):**

- Social login and OAuth providers
- Multi-factor authentication
- Password recovery flows

## Deliverables

1. Account registration and login experience
2. Persistent session management
3. Secure credential handling
4. Clean session termination

## Acceptance Criteria

- A new user can create an account and immediately access the application
- A returning user can authenticate and resume their previous session
- A logged-out session reveals no user data on subsequent visits
- Credentials are never stored or transmitted in plaintext

## Assumptions & Constraints

- Users have a valid email address for registration
- The application runs in a modern web browser
- No existing user data needs migration

This file is used as context during `decree plan` when generating SOW intents.
Specs may reference the SOW for traceability, but the SOW itself is oriented
around business outcomes rather than technical implementation details.
```

### `.decree/plans/spec.md` — Spec Template

This file is injected into the planning prompt (spec 09) when the user
runs `decree plan`. It teaches the AI how to write good specs.

```markdown
# Spec Template

Each spec is a self-contained unit of work. Specs are immutable — once created,
they are processed exactly once and never modified.

## Format

---

## routine: develop

# NN: Title

## Overview

Brief description of what this spec accomplishes.

## Requirements

Detailed technical requirements.

## Files to Modify

- path/to/file.rs — description of changes

## Acceptance Criteria

Write acceptance criteria as BDD-style **Given / When / Then** statements.
Each criterion describes a single testable behaviour that can be directly
translated into an automated test.

- **Given** [a precondition or initial state]
  **When** [an action or event occurs]
  **Then** [an observable, verifiable outcome]

### Guidelines

- One behaviour per criterion — if you need "And" more than once, split
  into separate criteria.
- **Given** sets up state: configuration, data, environment. Be specific
  enough that a test can reproduce it.
- **When** is a single action: a command invocation, a function call, a
  user interaction.
- **Then** is an assertion: what changed, what was produced, what was
  returned. Must be objectively verifiable — no "should work correctly".
- Cover the happy path, key error cases, and edge cases.
- Each criterion maps to one or more test functions. Name tests after the
  scenario they verify.

### Example

- **Given** a project with no `.decree/` directory
  **When** the user runs `decree init`
  **Then** `.decree/` is created with subdirectories: `routines/`, `plans/`,
  `cron/`, `inbox/` (with `done/`), `runs/`, `venv/`

- **Given** the model file does not exist at the configured path
  **When** `decree init` finishes provider selection
  **Then** the user is prompted to download the model
  **And** if declined, the manual download URL is printed and init continues

- **Given** `decree init` has already been run in this directory
  **When** the user runs `decree init` again
  **Then** existing files are not overwritten and a warning is printed

## Rules

- **Naming**: `NN-descriptive-name.spec.md` (e.g., `01-add-auth.spec.md`)
- **Frontmatter**: Optional YAML with `routine:` field (defaults to develop)
- **Ordering**: Alphabetical by filename determines execution order
- **Immutability**: Never edit a processed spec — create a new one instead
- **Self-contained**: Each spec should be independently implementable
- **Day-sized**: Each spec should be completable in one day or less of
  focused work
- **Testable**: Every acceptance criterion must be verifiable by an
  automated test

This file is used as context during `decree plan` when generating specs.
```

### Default Execution Routines

Routines live in `.decree/routines/` and come in two formats: **shell
scripts** (`.sh`) and **Jupyter notebooks** (`.ipynb`). Both do the same
job: read the spec or task, prompt an AI to implement it, then verify the
work. **Every routine template must be provided in both formats.**

`decree init` always creates the `.sh` versions of all built-in routines
(`develop.sh`, `rust-develop.sh`). When notebook support is enabled, it
also creates the `.ipynb` versions — and notebooks take precedence over
shell scripts when both exist for the same routine name (see spec 06
for discovery rules).

Users can edit either format or add new routines in either format. The AI
command should match the provider selected during init (the default below
uses Claude CLI).

Built-in routines:

- **develop** — General-purpose: delegates to AI, then verifies
- **rust-develop** — Rust-specific: delegates to AI, runs `cargo build
--release` and `cargo test`, then hands failures to a QA AI for fix-up

### `.decree/routines/develop.sh` — Default Shell Script Routine

```bash
#!/usr/bin/env bash
# Develop
#
# Default routine that delegates work to an AI assistant.
# Reads the spec file or task message, prompts the AI to implement all
# requirements, then verifies acceptance criteria are met.
set -euo pipefail

# Parameters (decree injects these as env vars)
spec_file="${spec_file:-}"
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"

# Determine the work description
if [ -n "$spec_file" ] && [ -f "$spec_file" ]; then
    WORK_FILE="$spec_file"
else
    WORK_FILE="$message_file"
fi

# Implementation
claude -p "You are a senior software engineer. Read the work description at ${WORK_FILE} and implement all requirements. Follow best practices: clean code, proper error handling, and tests where appropriate. Work methodically through each requirement." \
  --allowedTools 'Edit,Write,Bash(cargo*),Bash(npm*),Bash(python*),Bash(make*)'

# Verification
claude -p "Read the work description at ${WORK_FILE}. Verify that all requirements and acceptance criteria are met. Run any tests defined in the project. Clean up unused code: remove dead imports, unused functions, unreachable branches, and orphaned helpers introduced during implementation. Report what passes and what fails. Exit 0 if everything passes, exit 1 if anything fails." \
  --allowedTools 'Edit,Write,Bash(cargo*),Bash(npm*),Bash(python*),Bash(make*)'
```

The shell script template in `src/templates/develop.sh` uses `{AI_CMD}`
and `{ALLOWED_TOOLS}` as placeholders, replaced during `decree init`.
Parameters are injected as environment variables — the `${var:-default}`
pattern lets scripts run standalone while decree overrides values at
execution time. See spec 06 for the full shell script routine format,
parameter discovery, and description extraction conventions.

### `.decree/routines/develop.ipynb` — Default Notebook Routine

This is a Jupyter notebook (`.ipynb`) with the Python kernel. The notebook
has four cells: a markdown description, a parameters cell, an
implementation cell, and a verification cell.

**Cell 1 — Markdown description:**

```markdown
# Develop

Default routine that delegates work to an AI assistant.
Reads the spec file or task message, prompts the AI to implement all
requirements, then verifies acceptance criteria are met.

## Parameters

The first code cell is tagged `parameters` (papermill convention).
Decree injects values via papermill; when running manually in VS Code,
fill them in by hand.
```

**Cell 2 — Parameters (tagged `["parameters"]` in cell metadata):**

```python
input_file = ""          # Path to the input file (spec or task; empty if inline)
message_file = ""        # Path to the message file in the run directory
message_id = ""          # Full message ID (chain-seq)
message_dir = ""         # This message's run directory
chain = ""               # Chain ID
seq = ""                 # Sequence number in chain
```

**Cell 3 — Implementation:**

```python
%%bash -s "$input_file" "$message_file"
# Determine the work description: spec file for spec messages, message file for tasks
if [ -n "$1" ] && [ -f "$1" ]; then
    WORK_FILE="$1"
else
    WORK_FILE="$2"
fi

claude -p "You are a senior software engineer. Read the work description at ${WORK_FILE} and implement all requirements. Follow best practices: clean code, proper error handling, and tests where appropriate. Work methodically through each requirement." \
  --allowedTools 'Edit,Write,Bash(cargo*),Bash(npm*),Bash(python*),Bash(make*)'
```

**Cell 4 — Verification:**

```python
%%bash -s "$input_file" "$message_file"
if [ -n "$1" ] && [ -f "$1" ]; then
    WORK_FILE="$1"
else
    WORK_FILE="$2"
fi

claude -p "Read the work description at ${WORK_FILE}. Verify that all requirements and acceptance criteria are met. Run any tests defined in the project. Clean up unused code: remove dead imports, unused functions, unreachable branches, and orphaned helpers introduced during implementation. Report what passes and what fails. Exit 0 if everything passes, exit 1 if anything fails." \
  --allowedTools 'Edit,Write,Bash(cargo*),Bash(npm*),Bash(python*),Bash(make*)'
```

The notebook is stored as a standard `.ipynb` JSON file. The parameters
cell must have `"tags": ["parameters"]` in its `metadata` so papermill
can inject values. The notebook template in `src/templates/develop.ipynb`
uses `{AI_CMD}` and `{ALLOWED_TOOLS}` as placeholders. During `decree init`,
these are replaced with the actual command and tools for the selected
AI provider (e.g., `claude -p` and the `--allowedTools` value for Claude CLI).

### `.decree/routines/rust-develop.sh` — Rust Shell Script Routine

```bash
#!/usr/bin/env bash
# Rust Develop
#
# Rust-specific development routine. Delegates implementation to an AI
# assistant, builds and tests the result, then hands build/test output
# to a QA engineer AI to diagnose and fix any failures. Use this routine
# for Rust projects where you want cargo build --release and cargo test
# as the verification gate with automated fix-up.
set -euo pipefail

# Parameters (decree injects these as env vars)
spec_file="${spec_file:-}"
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"

# Determine the work description
if [ -n "$spec_file" ] && [ -f "$spec_file" ]; then
    WORK_FILE="$spec_file"
else
    WORK_FILE="$message_file"
fi

# Step 1: Implementation
claude -p "You are a senior Rust engineer. Read the work description at ${WORK_FILE}.
Implement all requirements. Follow Rust best practices: proper error
handling with Result/Option, no unwrap in library code, idiomatic types,
and tests where appropriate. Work methodically through each requirement." \
  --allowedTools 'Read,Edit,Write,Bash(cargo*),Bash(python*),Bash(make*),Bash(npm*)'

# Step 2: Build and test
echo "=== Building (release) ==="
cargo build --release 2>&1 | tee "${message_dir}/build.log" || true

echo "=== Running tests ==="
cargo test 2>&1 | tee "${message_dir}/test-output.log" || true

# Step 3: QA — diagnose and fix failures
claude -p "You are an expert Quality Assurance Engineer for a Rust project.

Read the work description at ${WORK_FILE}.
Read the build output at ${message_dir}/build.log.
Read the test output at ${message_dir}/test-output.log.

If the build or tests passed cleanly, verify the implementation meets all
requirements and acceptance criteria from the work description — exit 0.

If there are build errors or test failures:
1. Diagnose each failure — read the relevant source and test code.
2. Apply the minimal fix. Prefer fixing source code over weakening tests.
3. Do NOT add new features or refactor unrelated code.
4. Run cargo build --release and cargo test again to confirm everything passes.
Exit 0 only if the build succeeds and all tests pass." \
  --allowedTools 'Read,Edit,Write,Bash(cargo*),Bash(python*),Bash(make*),Bash(npm*)'
```

Same `{AI_CMD}` and `{ALLOWED_TOOLS}` placeholder rules as `develop.sh`.

### `.decree/routines/rust-develop.ipynb` — Rust Notebook Routine

Same structure as `develop.ipynb` but with the rust-develop steps. The
notebook has six cells: markdown description, parameters, implementation
(AI), build/test (cargo), QA markdown header, and QA (AI).

**Cell 1 — Markdown description:**

```markdown
# Rust Develop

Rust-specific development routine. Delegates implementation to an AI
assistant, builds and tests the result, then hands build/test output
to a QA engineer AI to diagnose and fix any failures. Use this routine
for Rust projects where you want cargo build --release and cargo test
as the verification gate with automated fix-up.
```

**Cell 2 — Parameters (tagged `["parameters"]`):**

```python
spec_file = ""
message_file = ""
message_id = ""
message_dir = ""
chain = ""
seq = ""
```

**Cell 3 — Python: determine work file:**

```python
import os

if spec_file and os.path.isfile(spec_file):
    WORK_FILE = spec_file
else:
    WORK_FILE = message_file
```

**Cell 4 — Implementation (bash):**

```python
%%bash -s "$WORK_FILE"
claude -p "You are a senior Rust engineer. Read the work description at $1.
Implement all requirements. Follow Rust best practices: proper error
handling with Result/Option, no unwrap in library code, idiomatic types,
and tests where appropriate. Work methodically through each requirement." \
  --allowedTools 'Read,Edit,Write,Bash(cargo*),Bash(python*),Bash(make*),Bash(npm*)'
```

**Cell 5 — Build and test (bash):**

```python
%%bash -s "$message_dir"
set -euo pipefail
echo "=== Building (release) ==="
cargo build --release 2>&1 | tee "$1/build.log" || true

echo "=== Running tests ==="
cargo test 2>&1 | tee "$1/test-output.log" || true
```

**Cell 6 — QA (bash):**

```python
%%bash -s "$WORK_FILE" "$message_dir"
claude -p "You are an expert Quality Assurance Engineer for a Rust project.

Read the work description at $1.
Read the build output at $2/build.log.
Read the test output at $2/test-output.log.

If the build or tests passed cleanly, verify the implementation meets all
requirements and acceptance criteria from the work description — exit 0.

If there are build errors or test failures:
1. Diagnose each failure — read the relevant source and test code.
2. Apply the minimal fix. Prefer fixing source code over weakening tests.
3. Do NOT add new features or refactor unrelated code.
4. Run cargo build --release and cargo test again to confirm everything passes.
Exit 0 only if the build succeeds and all tests pass." \
  --allowedTools 'Read,Edit,Write,Bash(cargo*),Bash(python*),Bash(make*),Bash(npm*)'
```

Same `{AI_CMD}` and `{ALLOWED_TOOLS}` placeholder rules as `develop.ipynb`.

### `.decree/.gitignore`

```
venv/
inbox/
runs/
sessions/
last-run.yml
```

## Acceptance Criteria

- **Given** init completes with notebook support disabled (default)
  **When** inspecting `.decree/routines/`
  **Then** `develop.sh` and `rust-develop.sh` exist
  **And** no `.ipynb` files exist

- **Given** init completes with notebook support enabled
  **When** inspecting `.decree/routines/`
  **Then** both `.sh` and `.ipynb` versions of every built-in routine exist
  (`develop.sh`, `develop.ipynb`, `rust-develop.sh`, `rust-develop.ipynb`)

- **Given** init completes successfully
  **When** inspecting `.decree/routines/develop.sh`
  **Then** it is an executable bash script with a description comment block,
  parameter declarations using `${var:-}` defaults, implementation, and
  verification steps

- **Given** init completes successfully
  **When** inspecting `.decree/routines/rust-develop.sh`
  **Then** it is an executable bash script with implementation (AI),
  build/test (`cargo build --release`, `cargo test`), and QA (AI) steps

- **Given** init completes with notebook support enabled
  **When** inspecting `.decree/routines/develop.ipynb`
  **Then** it is a valid `.ipynb` with a markdown cell, a parameters cell
  (tagged `["parameters"]`), an implementation cell, and a verification cell
  **And** the parameters cell declares `spec_file`, `message_file`,
  `message_id`, `message_dir`, `chain`, `seq` as empty strings

- **Given** init completes with notebook support enabled
  **When** inspecting `.decree/routines/rust-develop.ipynb`
  **Then** it is a valid `.ipynb` with parameters, implementation,
  build/test, and QA cells matching the rust-develop.sh logic

- **Given** init completes successfully
  **When** inspecting `.decree/plans/sow.md`
  **Then** it contains the full SOW template with structure, writing
  guidelines, and example

- **Given** init completes successfully
  **When** inspecting `.decree/plans/spec.md`
  **Then** it contains the spec template with format example and rules

- **Given** init completes successfully
  **When** inspecting `.decree/.gitignore`
  **Then** it includes `venv/`, `inbox/`, `runs/`, `sessions/`, and `last-run.yml`

- **Given** the decree source tree
  **When** inspecting `src/templates/`
  **Then** it contains `sow.md`, `spec.md`, `gitignore`, `develop.sh`,
  `develop.ipynb`, `rust-develop.sh`, and `rust-develop.ipynb`
  **And** each is embedded via `include_str!()` at compile time
  **And** all routine templates use `{AI_CMD}` and `{ALLOWED_TOOLS}`
  placeholders that are replaced during `decree init` based on the selected
  AI provider
  **And** `.ipynb` templates are only written to `.decree/routines/` when
  notebook support is enabled

- **Given** the user selected Claude CLI as the planning AI during init
  **When** inspecting the develop routine's implementation and verification cells
  **Then** they use `claude -p` with appropriate prompts and `--allowedTools`
