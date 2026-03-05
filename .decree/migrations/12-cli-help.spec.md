# 12: CLI Help System

## Overview

`decree help` outputs a detailed, verbose explanation of everything
decree does. This is more than a typical `--help` flag — it's a
built-in reference that explains the message format, processing
pipeline, routine system, routine authoring, cron scheduling, and
how to get started.

## `decree help`

Prints comprehensive help to stdout. The output covers:

### Section 1: What Decree Does

```
Decree is an AI orchestrator for structured, reproducible workflows.

It processes migration files through AI-powered routines, with optional
git stash hooks for change tracking and retry on failure.

Core workflow:
  1. Write migration files describing work in .decree/migrations/
  2. Run `decree process` to execute them through AI routines
  3. Routines invoke AI tools (opencode, claude, copilot) to do the work
  4. Optional git stash hooks isolate each routine's changes
```

### Section 2: Commands

```
Commands:
  decree process              Process all pending migrations + drain inbox
  decree prompt [NAME]        Build prompt from template, copy or launch AI
  decree routine              List routines (interactive select + run)
  decree routine <name>       Show routine detail + run pre-checks
  decree verify               Run all routine pre-checks
  decree daemon [--interval]  Continuous inbox + cron monitoring
  decree status               Show processing progress
  decree log [ID]             Show routine execution output
  decree init                 Initialize a new decree project
  decree help                 This help text
```

### Section 3: Message Format

```
Message Format:
  Messages use optional YAML frontmatter followed by a markdown body.
  Frontmatter fields control routing and parameters.

  ---
  routine: develop              # Which routine to execute
  custom_field: value           # Custom fields become env vars
  ---
  Description of the work to do.

  The body can be empty. All frontmatter fields are optional — decree
  fills in missing fields automatically (chain, seq, id, routine).
  Migration content is copied into the message body when processed.
```

### Section 4: How Messages Are Processed

```
Processing Pipeline:
  1. Migration files in .decree/migrations/ are read in alphabetical order
  2. Each migration becomes an inbox message in .decree/inbox/
  3. Messages are normalized (missing fields filled, routine selected)
  4. Lifecycle hooks run (beforeEach — e.g. git stash baseline)
  5. The selected routine executes with parameters as env vars
  6. On success: afterEach hook runs, message deleted from inbox (run dir is the record)
  7. On failure: retry strategy applies (hooks handle state management)
  8. After all retries: dead-letter the message
  9. Follow-up messages from routines are processed depth-first
  10. Inbox is fully drained before the next migration starts
```

### Section 5: How to Define Routines

```
Defining Routines:
  Routines are shell scripts in .decree/routines/ (nested dirs allowed).

  Required structure:
    #!/usr/bin/env bash
    # Title
    #
    # Description shown by `decree routine`.
    set -euo pipefail

    # --- Parameters ---
    # message_file  - Path to message.md in the run directory
    # message_id    - Full message ID (e.g., D0001-1432-01-add-auth-0)
    # message_dir   - Run directory path (contains logs from prior attempts)
    # chain         - Chain ID (D<NNNN>-HHmm-<name>)
    # seq           - Sequence number
    message_file="${message_file:-}"
    message_id="${message_id:-}"
    message_dir="${message_dir:-}"
    chain="${chain:-}"
    seq="${seq:-}"

    # Pre-check (required — exit 0 if ready, non-zero if not):
    if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then
        command -v opencode >/dev/null 2>&1 || { echo "opencode not found" >&2; exit 1; }
        exit 0
    fi

    # Custom params (from frontmatter, discovered automatically):
    my_param="${my_param:-default}"

    # Implementation (invoke AI tool to do the work):
    opencode run "Read ${message_file} and implement the requirements.
    Previous attempt logs (if any) are in ${message_dir} for context."

  Custom parameter discovery:
    Decree scans for var="${var:-default}" patterns after the pre-check
    block. Standard params are excluded. Remaining are custom parameters
    whose values come from message frontmatter.

  Tips:
    - Pre-check failures should print to stderr
    - Use ${message_dir} for prior attempt logs as AI context on retries
    - Use {AI_CMD} placeholder in templates (replaced at init time)
    - Routines are non-interactive — only `decree prompt` launches interactive AI
    - Use --no-color flag or NO_COLOR env var to disable color output

  Run `decree verify` to check all routines' pre-checks at once.
  Run `decree prompt routine` for an AI-assisted routine authoring guide.
```

### Section 6: Lifecycle Hooks

```
Lifecycle Hooks (config.yml):
  hooks:
    beforeAll: ""      # Routine to run before all processing
    afterAll: ""       # Routine to run after all processing
    beforeEach: ""     # Routine to run before each message
    afterEach: ""      # Routine to run after each message

  Hooks receive additional env vars:
    DECREE_HOOK            — hook type name
    DECREE_ATTEMPT         — current attempt number (beforeEach/afterEach)
    DECREE_MAX_RETRIES     — configured max retries (beforeEach/afterEach)
    DECREE_ROUTINE_EXIT_CODE — routine exit code (afterEach only)
```

### Section 7: Cron Scheduling

```
Cron Scheduling:
  Place .md files with a `cron` frontmatter field in .decree/cron/:

  ---
  cron: "0 9 * * 1-5"
  routine: develop
  ---
  Run the weekday morning task.

  Common schedules:
    * * * * *       Every minute
    0 * * * *       Every hour
    0 9 * * *       Daily at 9:00 AM
    0 9 * * 1-5     Weekdays at 9:00 AM
    0 0 * * 0       Weekly on Sunday
    0 0 1 * *       Monthly on the 1st
    */15 * * * *    Every 15 minutes

  Run `decree daemon` to start monitoring cron and inbox.
```

### Section 8: Getting Started

```
Getting Started:
  1. decree init                    # Set up project
  2. decree prompt migration        # Plan work with AI → migration files
  3. decree verify                  # Check routines are ready
  4. decree process                 # Execute all migrations
  5. decree routine                 # Run individual routines interactively
  6. decree prompt routine          # Get AI help building new routines
```

## Implementation

The help text is a single `include_str!()` template in `src/templates/help.txt`
or constructed programmatically. It is printed to stdout with no paging
(users can pipe to `less` if needed).

`decree --help` (clap's built-in) shows the short form. `decree help`
(the subcommand) shows the verbose form described above.

## `decree --version` / `decree -v`

Prints the version and exits:

```
decree 0.2.0
```

The version is read from `Cargo.toml` at compile time via the clap
`#[command(version)]` attribute (which uses `CARGO_PKG_VERSION`).

## Acceptance Criteria

- [ ] `decree help` prints verbose multi-section help to stdout
- [ ] Help explains what decree does and the core workflow
- [ ] Help lists all available commands with descriptions
- [ ] Help explains message format with frontmatter example
- [ ] Help documents that migration content is copied into message body
- [ ] Help explains the full processing pipeline
- [ ] Help shows how to define routines with required structure
- [ ] Help documents pre-check sections (stderr output)
- [ ] Help documents `${message_dir}` for prior attempt context
- [ ] Help mentions `{AI_CMD}` placeholder for templates
- [ ] Help documents that routines are non-interactive
- [ ] Help references `decree prompt routine` for AI-assisted authoring
- [ ] Help documents lifecycle hooks with env vars
- [ ] Help documents cron scheduling with common expression examples
- [ ] Help includes getting started steps
- [ ] `decree --help` shows short clap-generated help (separate from `decree help`)
- [ ] `decree --version` / `decree -v` prints version from `Cargo.toml` and exits
