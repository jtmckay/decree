# 02: Default Templates

## Overview

All files `decree init` writes into a new project. Prompt templates are
plain markdown files the user can customize. Routine templates define
default execution workflows. Source files live in `src/templates/` and
are embedded at compile time via `include_str!()`. Routine templates use
`{AI_CMD}` placeholder replaced at init time based on the detected AI
backend (from `commands.ai_command` in config).

## Template Embedding

Templates embedded from `src/templates/` at compile time:
- `sow.md` — statement of work prompt (prompt)
- `migration.md` — migration template (prompt)
- `routine.md` — routine authoring guide (prompt)
- `router.md` — routine selection prompt (internal, not in prompts/)
- `develop.sh` — default routine
- `rust-develop.sh` — Rust-specific routine
- `gitignore` — .decree/.gitignore

## `.decree/prompts/sow.md` — Statement of Work Prompt

Used by `decree prompt sow` to guide the AI in creating a statement of
work that captures business intent.

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
```

## `.decree/prompts/migration.md` — Migration Template

Used by `decree prompt migration` to guide the AI in creating migration files.

```markdown
# Migration Template

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

    ## Files to Modify
    - path/to/file.rs — description of changes

    ## Acceptance Criteria
    Write acceptance criteria as BDD-style Given / When / Then statements.

## Acceptance Criteria Guidelines

- One behaviour per criterion — if you need "And" more than once, split
  into separate criteria.
- **Given** sets up state: configuration, data, environment. Be specific
  enough that a test can reproduce it.
- **When** is a single action: a command invocation, a function call, a
  user interaction.
- **Then** is an assertion: what changed, what was produced, what was
  returned. Must be objectively verifiable — no "should work correctly".
- Cover the happy path, key error cases, and edge cases.

### Example

- **Given** a project with no `.decree/` directory
  **When** the user runs `decree init`
  **Then** `.decree/` is created with all expected subdirectories

- **Given** `decree init` has already been run in this directory
  **When** the user runs `decree init` again
  **Then** existing files are not overwritten and a warning is printed

## Rules

- **Naming**: `NN-descriptive-name.md` (e.g., `01-add-auth.md`)
- **Frontmatter**: Optional YAML with `routine:` field (defaults to develop)
- **Ordering**: Alphabetical by filename determines execution order
- **Immutability**: Never edit a processed migration — create a new one
- **Self-contained**: Each migration should be independently implementable
- **Day-sized**: Each migration should be completable in one day or less
- **Testable**: Every acceptance criterion must be verifiable by an automated test

## Existing Migrations

{migrations}

## Processed

{processed}
```

## `.decree/prompts/routine.md` — Routine Authoring Guide

Used by `decree prompt routine` to prime an AI for building new routines.
Contains the full routine authoring documentation (content from the
routine authoring section of `decree help` — see migration 12).

```markdown
# Routine Authoring Guide

A routine is a shell script in `.decree/routines/` that decree executes
with env vars populated from message frontmatter and runtime context.
Routines invoke AI tools to perform work. They can be nested in
subdirectories for organization.

## Required Structure

Every routine must follow this structure:

    #!/usr/bin/env bash
    # Title
    #
    # Short description (shown in `decree routine` list).
    # Additional lines shown in detail view.
    set -euo pipefail

    # --- Parameters ---
    # message_file  - Path to message.md in the run directory
    # message_id    - Full message ID (e.g., D0001-1432-01-add-auth-0)
    # message_dir   - Run directory path (contains logs from prior attempts)
    # chain         - Chain ID
    # seq           - Sequence number
    message_file="${message_file:-}"
    message_id="${message_id:-}"
    message_dir="${message_dir:-}"
    chain="${chain:-}"
    seq="${seq:-}"

    # Pre-check (required — exit 0 if ready, non-zero if not):
    if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then
        command -v {AI_CMD} >/dev/null 2>&1 || { echo "{AI_CMD} not found" >&2; exit 1; }
        exit 0
    fi

    # Custom params (from frontmatter, discovered automatically):
    my_param="${my_param:-default}"

    # --- Implementation ---
    {AI_CMD} run "Read ${message_file} and implement the requirements.
    Previous attempt logs (if any) are in ${message_dir} for context."

## Standard Parameter Mapping

| Frontmatter field | Env var in routine | Notes |
|---|---|---|
| *(auto)* | `message_file` | Path to message.md in run directory |
| *(auto)* | `message_id` | Unique message identifier |
| *(auto)* | `message_dir` | Run directory path |
| `chain` | `chain` | Chain ID (`D<NNNN>-HHmm-<name>`) |
| `seq` | `seq` | Sequence number in chain |

## Custom Parameter Discovery

Decree scans the routine from top to bottom:
1. Skips: shebang, comments, blanks, `set` builtins, pre-check block
2. Matches: `var_name="${var_name:-default_value}"`
3. Stops at first non-matching line
4. Excludes standard parameter names
5. Remaining variables are custom parameters
6. Empty defaults (`${var:-}`) mean optional with no default

Custom values come from message frontmatter — any key not in the
standard set is passed as an env var.

## Pre-Check Section

Every routine must include a pre-check gate:
- Gate on `DECREE_PRE_CHECK=true` env var
- Place after standard params, before custom params
- Exit 0 = routine is ready, exit non-zero = not ready
- Print missing dependency to **stderr** on failure
- Used by `decree routine <name>` and `decree verify`

## Nested Routines

Routines can be organized in subdirectories:

    .decree/routines/
    ├── develop.sh           # routine: develop
    ├── deploy/
    │   ├── staging.sh       # routine: deploy/staging
    │   └── production.sh    # routine: deploy/production
    └── review/
        └── pr.sh            # routine: review/pr

## Tips

- **Pre-check required**: Every routine must have a pre-check section
- **Parameter comments**: Use `# --- Parameters ---` block to document vars
- **Optional marker**: Mark optional params with "Optional." in the comment
- **Discovery boundary**: Use a comment like `# --- Implementation ---`
- **Default values**: Use meaningful defaults where possible
- **`set -euo pipefail`**: Always include — decree expects non-zero on failure
- **Run directory**: Use `${message_dir}` for logs and context from prior attempts
- **AI-specific**: Routines should invoke an AI tool — they are not
  general-purpose shell scripts

## Available Routines

{routines}
```

## `.decree/router.md` — Routine Selection Prompt

**Internal to decree.** Lives at `.decree/router.md` (not in `prompts/`)
so it is not selectable by `decree prompt`. Used during message
normalization when a message has no `routine:` field. Decree populates
`{routines}` and `{message}` before sending to `commands.ai_router`.

```markdown
Select the most appropriate routine for the given message.

## Available Routines

{routines}

## Message

{message}

## Instructions

Respond with ONLY the routine name (e.g., `develop`). No explanation.
If no routine is a clear match, respond with `develop`.
```

## `.decree/routines/develop.sh`

```bash
#!/usr/bin/env bash
# Develop
#
# Default routine that delegates work to an AI assistant.
# Reads the task message, prompts the AI to implement all
# requirements, then verifies acceptance criteria are met.
set -euo pipefail

# --- Parameters ---
# message_file  - Path to message.md in the run directory
# message_id    - Full message ID (e.g., D0001-1432-01-add-auth-0)
# message_dir   - Run directory path
# chain         - Chain ID
# seq           - Sequence number
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"

# Pre-check: verify AI tool is available
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then
    command -v {AI_CMD} >/dev/null 2>&1 || { echo "{AI_CMD} not found" >&2; exit 1; }
    exit 0
fi

# Implementation
{AI_CMD} run "Read ${message_file} and implement all requirements.
Previous attempt logs (if any) are in ${message_dir} for context.
Follow best practices: clean code, proper error handling, and tests
where appropriate."

# Verification
{AI_CMD} run "Read ${message_file}. Verify that all requirements and
acceptance criteria are met. Run any tests. Report what passes and what
fails. Exit 0 if everything passes, exit 1 if anything fails."
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
# message_id    - Full message ID (e.g., D0001-1432-01-add-auth-0)
# message_dir   - Run directory path
# chain         - Chain ID
# seq           - Sequence number
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"

# Pre-check: verify AI tool and cargo are available
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then
    command -v {AI_CMD} >/dev/null 2>&1 || { echo "{AI_CMD} not found" >&2; exit 1; }
    command -v cargo >/dev/null 2>&1 || { echo "cargo not found" >&2; exit 1; }
    exit 0
fi

# Step 1: Implementation
{AI_CMD} run "You are a senior Rust engineer. Read ${message_file} and
implement all requirements with proper error handling and tests.
Previous attempt logs (if any) are in ${message_dir} for context."

# Step 2: Build and test
echo "=== Building (release) ==="
cargo build --release 2>&1 | tee "${message_dir}/build.log" || true
echo "=== Running tests ==="
cargo test 2>&1 | tee "${message_dir}/test-output.log" || true

# Step 3: QA
{AI_CMD} run "Read ${message_file}, build output at ${message_dir}/build.log,
test output at ${message_dir}/test-output.log. Fix any failures. Run cargo
build --release and cargo test again. Exit 0 only if everything passes."
```

## `.decree/.gitignore`

```
inbox/
outbox/
runs/
```

## Acceptance Criteria

- [ ] `decree init` creates `develop.sh` and `rust-develop.sh` in `.decree/routines/`
- [ ] Both routines use `{AI_CMD}` placeholder replaced with `commands.ai_command` value
- [ ] Both routines include a pre-check section gated on `DECREE_PRE_CHECK`
- [ ] Pre-check failures print to stderr (not stdout)
- [ ] `.decree/prompts/sow.md` contains the statement of work prompt
- [ ] `.decree/prompts/migration.md` contains the migration template
- [ ] `.decree/prompts/routine.md` contains the routine authoring guide
- [ ] `.decree/router.md` is placed at `.decree/router.md` (not in `prompts/`)
- [ ] `.decree/router.md` contains the routine selection prompt with `{routines}` and `{message}` placeholders
- [ ] `.decree/.gitignore` excludes `inbox/`, `outbox/`, and `runs/`
- [ ] Template source files exist in `src/templates/` and are embedded via `include_str!()`
- [ ] Routine templates have description comment headers for `decree routine` extraction
- [ ] Routine templates reference `${message_dir}` for prior attempt context
