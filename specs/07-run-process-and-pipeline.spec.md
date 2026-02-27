---
routine: rust-develop
---

# 07: Run, Process, and Pipeline

## Overview

The commands that create and execute messages. `decree run` creates and
processes a single message (interactively or via flags). `decree process`
batch-processes all unprocessed specs. Both share the core message
processing loop which handles checkpoint creation (spec 04), routine
execution (spec 06), retry strategy, dead-lettering, and depth-first
follow-up processing.

## Requirements

### `decree run` — Single Message

```
decree run -p PROMPT                            # derive name from prompt
decree run -m NAME -p PROMPT                    # explicit name
decree run -p PROMPT -v routine=review          # specify routine
decree run -v input_file=specs/01.spec.md       # run a spec
decree run -m NAME -v KEY=VALUE -v KEY=VALUE    # pass any frontmatter
echo "task" | decree run                        # piped input
decree run                                      # interactive mode (TTY only)
```

#### Flags

| Flag | Description | Required |
|------|-------------|----------|
| `-m NAME` | Message name — becomes the `.md` filename in the inbox (kebab-cased, truncated to 50 chars). If omitted, derived from the prompt via the router AI (see below). | No |
| `-p PROMPT` | Task prompt — the message body. Or pipe via STDIN. | No |
| `-v KEY=VALUE` | Frontmatter field — repeatable. Sets any field: `routine`, `input_file`, custom variables, etc. | No |

There are no positional arguments. Spec files, routine selection, and
custom variables are all passed as `-v` pairs. This keeps the interface
uniform — every frontmatter field is set the same way.

#### Message Name Derivation

When `-m` is omitted in non-interactive mode, the message name is derived
automatically using the router AI (`commands.router`):

1. Send the prompt/body text to the router with the instruction:
   `"Summarize this task as a kebab-case name under 5 words. Respond with ONLY the name."`
2. Parse the response, kebab-case it, truncate to 50 chars
3. Check for collisions against existing `.decree/inbox/` and
   `.decree/inbox/done/` filenames. If a collision is found, append a
   numeric suffix (`-2`, `-3`, etc.)

If the router fails or no prompt is provided, fall back to generating a
name from the chain ID (e.g. `run-2025022514320000`).

#### Non-interactive mode (flags provided)

When `-p`, `-v`, or piped STDIN is given, decree runs non-interactively.
**No prompts are ever shown** — all inputs come from flags and defaults.

Input resolution:
1. If `-p <prompt>` is given: use the prompt string as the message body
2. If stdin is not a TTY (piped input): read stdin as the message body
3. If neither: the message has no body (valid for spec messages where
   `input_file` is the source of truth)

Processing:
1. Generate a new chain ID
2. Resolve message name: `-m NAME` if provided, else derive from prompt
   (see "Message Name Derivation")
3. Create an inbox message with `seq: 0`:
   - All `-v` pairs are written as YAML frontmatter fields
   - If `input_file` points to a `.spec.md`: `type` is set to `spec`
   - Otherwise: `type` is `task`
   - `-p`/STDIN content becomes the message body
4. Normalize the message (see spec 05 for normalization):
   - `routine` from `-v routine=X`, else spec frontmatter, else router AI,
     else config default
   - Standard fields (`chain`, `seq`, `id`, `type`) are derived
   - Any missing custom variables use the routine's defaults silently
5. Process the message immediately (see "Message Processing" below)
6. If the routine spawns follow-up messages, process them depth-first
7. Exit when inbox has no more messages for this chain

Example — task with custom variables:

```bash
decree run -m pr-review -p "Review the current branch" \
  -v routine=pr-review -v target_branch=main -v reviewer=alice
```

Creates an inbox message with frontmatter:

```yaml
---
id: 2025022514320000-0
chain: 2025022514320000
seq: 0
type: task
routine: pr-review
target_branch: main
reviewer: alice
---
Review the current branch
```

Example — spec file:

```bash
decree run -m add-auth -v input_file=specs/01-add-auth.spec.md
```

Creates a `type: spec` message with no body (the spec file is the source
of truth).

#### Interactive mode (no arguments, TTY)

When no arguments are provided and stdin is a TTY, `decree run` walks
through each field step by step:

1. **Select routine**: arrow-key selection with fuzzy filtering (per spec
   01 Interactive Selection UX). Config default is pre-highlighted. Required.
2. **Message name**: free-text name for this run (e.g. `fix-auth-types`).
   Required. Converted to kebab-case for the message filename.
3. **Input file**: path to a spec or other file. Optional — press Enter to
   skip. If provided and it ends in `.spec.md`, the message `type` is `spec`.
4. **Custom parameters**: for each extra parameter declared in the selected
   routine (beyond the standard set), prompt with the parameter name and
   its default value. Optional — press Enter to skip (uses the routine's
   default).
5. **Message body**: free-text description of the work to do. Required
   (unless `input_file` was provided — then optional, press Enter to skip).
   This becomes the markdown body of the inbox message (after frontmatter).
   Multi-line input: read until an empty line or EOF.
6. `chain`, `seq`, `message_id`, `message_dir`, `message_file` are derived
   automatically — never prompted.

After all prompts are answered, the message is created and processed as in
non-interactive mode.

#### Subsequent runs (interactive recall)

When `decree run` starts in interactive mode and a previous interactive run
exists (tracked in `.decree/last-run.yml`), the previous run's parameters
are suggested as defaults:

- Routine, message name, input file, and custom parameters are pre-filled
  from the last run — the user can press Enter to accept each or type a
  new value to override
- The **message body** is always entered fresh — it is never pre-filled

The `.decree/last-run.yml` file stores the last interactive run's parameters:

```yaml
routine: develop
message_name: fix-auth-types
input_file: specs/01-add-auth.spec.md
custom:
  target_branch: main
```

This file is written after each successful interactive run and read at the
start of the next one. It is not tracked in git (covered by `.decree/.gitignore`).

### `decree process` — Batch Spec Processing

```
decree process                           # process all unprocessed specs
```

`decree process`:
1. Read `specs/processed-spec.md` to find unprocessed specs
2. For the first unprocessed spec (alphabetically):
   - Generate a new chain ID
   - Create an inbox message (`type: spec`, `seq: 0`)
   - Process the message and any follow-up messages in its chain
   - On success: mark spec as processed in `processed-spec.md`
3. Move to the next unprocessed spec (new chain ID, new chain)
4. Repeat until all specs are processed or a spec fails

Each spec starts a new chain. If a spec fails all retries, its message is
dead-lettered and `decree process` continues to the next spec.

### Message Processing

The core processing logic is shared by `decree run`, `decree process`,
and `decree daemon` (spec 08):

```
take next message from inbox
  (prefer messages from the current chain — depth-first)
normalize message (fill missing fields — see spec 05)
create .decree/runs/<chain>-<seq>/
copy normalized message -> .decree/runs/<chain>-<seq>/message.md

1. Save checkpoint -> <msg-dir>/manifest.json  (spec 04)
2. Determine routine (from normalized message frontmatter)
3. Resolve routine file (see spec 06 for discovery rules)
4. Execute routine based on file extension:

   **Shell script** (`.sh`):
   bash .decree/routines/<routine>.sh
   - Environment variables: spec_file, message_file, message_id,
     message_dir, chain, seq, plus custom variables
   - Working directory: project root
   - Combined stdout+stderr captured to <msg-dir>/routine.log
   - No venv required

   **Notebook** (`.ipynb`):
   papermill .decree/routines/<routine>.ipynb <msg-dir>/output.ipynb \
     -p spec_file "<spec_file>" \
     -p message_file "<msg-dir>/message.md" \
     -p message_id "<chain>-<seq>" \
     -p message_dir "<msg-dir>" \
     -p chain "<chain>" \
     -p seq "<seq>" \
     [-p <custom_param> "<value>" ...]
   - stderr captured to <msg-dir>/papermill.log
   - Requires venv with papermill + ipykernel

5. On SUCCESS:
   - Generate changes.diff -> <msg-dir>/changes.diff  (spec 04)
   - Move message to .decree/inbox/done/
   - If type=spec: append spec filename to processed-spec.md
6. On FAILURE (see "Retry Strategy" below):
   - Generate changes.diff (partial work)
   - Retry according to the retry strategy
7. On EXHAUSTION (all retries failed):
   - Revert to the original checkpoint (pre-first-attempt state)
   - Move message to .decree/inbox/dead/
   - Continue to next message (do not halt)
8. Check inbox for new messages in this chain
   - If found and seq < max_depth: process depth-first (go to top)
   - If seq >= max_depth: move to .decree/inbox/dead/ with error note
```

**Depth-first processing**: after a routine completes, the processor checks
the inbox for new messages with the same chain ID before processing any
other chains.

### Retry Strategy

When a routine fails, the retry behaviour depends on which attempt it is:

**Attempts 1 through N-1** (not the last retry):
- Generate `changes.diff` capturing the partial work
- **Do not revert** — leave the changes in place
- Re-execute the routine. The AI can see the state of its previous
  attempt (broken code, failing tests, partial implementation) and learn
  from its mistakes.

**Attempt N** (the final retry, where N = `max_retries`):
- Generate `changes.diff` capturing the partial work
- **Revert to the original checkpoint** (the manifest from before the
  first attempt, not the previous retry)
- Write a failure context file to `<msg-dir>/failure-context.md`
  summarizing all previous attempt results (which retries failed, what
  errors occurred, what was tried)
- Re-execute the routine with a clean slate and the failure context
  available at `<msg-dir>/failure-context.md`. The routine can read this
  file to understand what went wrong in prior attempts.

**After all retries exhausted:**
- Revert to the original checkpoint (pre-first-attempt state) so the
  working tree is clean for subsequent messages
- Move the message to `.decree/inbox/dead/`
- The message's run directory (`.decree/runs/<chain>-<seq>/`) is
  preserved with all attempt diffs and logs for debugging

### Dead-Letter Directory

Messages that cannot be processed are moved to `.decree/inbox/dead/`
instead of `.decree/inbox/done/`. This includes:

- Messages that exhaust all retries
- Messages rejected for `MaxDepthExceeded`
- Messages that fail normalization (e.g. `RoutineNotFound`)

Dead-lettered messages retain their original filename. The run directory
preserves all artifacts from every attempt. Users can inspect the failure,
fix the issue, and re-queue the message by moving it back to
`.decree/inbox/`.

### Depth Limiting

The sequence number acts as the depth counter:
- Root messages have `seq: 0`
- Each follow-up increments by 1
- When `seq >= max_depth` (default: 10), the message is rejected with a
  `MaxDepthExceeded` error and moved to `inbox/dead/` with an error note

## Acceptance Criteria

### Inbox and Execution

- **Given** the user runs `decree run -v input_file=specs/01-add-auth.spec.md`
  **When** the command executes
  **Then** a `type: spec` inbox message is created with `input_file` set
  **And** the message name is derived from the spec filename
  **And** a directory `.decree/runs/<chain>-0/` is created with all artifacts

- **Given** the user runs `decree run -m fix-types -p "fix the auth types"`
  **When** the command executes
  **Then** a `type: task` inbox message named `fix-types` is created with the
  prompt as the body

- **Given** the user runs `decree run -p "fix the auth type errors"`
  without `-m`
  **When** the command executes
  **Then** the router AI derives a message name (e.g. `fix-auth-type-errors`)
  **And** the name does not collide with existing inbox messages

- **Given** the user runs `decree run -p "fix" -v routine=develop`
  **When** the command executes
  **Then** `routine: develop` is set in frontmatter, skipping router
  selection

- **Given** the user runs `decree run -p "review" -v routine=pr-review -v target_branch=main`
  **When** the command executes
  **Then** all `-v` pairs appear as frontmatter fields in the inbox message
  **And** they are passed to the routine as parameters

- **Given** non-interactive mode and a routine declares `target_branch`
  **When** `target_branch` was not provided via `-v`
  **Then** the routine's default value is used silently (no prompt ever)

- **Given** the user runs `decree run` with no arguments and stdin is a TTY
  **When** the command executes
  **Then** interactive mode begins: routine selection, message name, input
  file, custom parameters, message body
  **And** each prompt is skippable except routine, message name, and message
  body (required unless input_file was provided)

- **Given** interactive mode prompts for routine selection
  **When** multiple routines exist in `.decree/routines/`
  **Then** an arrow-key selector is shown with fuzzy type-ahead filtering
  **And** the config default routine is pre-highlighted

- **Given** interactive mode prompts for custom parameters
  **When** the selected routine declares extra parameters (e.g. `target_branch`)
  **Then** each custom parameter is prompted with its name and default value
  **And** pressing Enter skips the parameter (uses routine default)

- **Given** interactive mode prompts for message body
  **When** the user enters text
  **Then** the text becomes the markdown body of the inbox message
  **And** multi-line input is accepted (terminated by empty line or EOF)

- **Given** a previous interactive run completed successfully
  **When** `decree run` starts in interactive mode again
  **Then** routine, message name, input file, and custom parameters are
  pre-filled from `.decree/last-run.yml`
  **And** the user can press Enter to accept each default or type a new value
  **And** the message body is always entered fresh (never pre-filled)

- **Given** a routine writes `<chain>-1.md` to the inbox
  **When** the `-0` message's routine completes
  **Then** the `-1` message is processed immediately (depth-first)
  **And** `.decree/runs/<chain>-1/` is created before any other chain

- **Given** message `-1` spawns message `-2`, which spawns `-3`
  **When** the chain is processed
  **Then** directories `-0`, `-1`, `-2`, `-3` exist in `.decree/runs/`
  **And** they were processed in that order (depth-first)

- **Given** a message has `seq >= max_depth`
  **When** the processing loop encounters it
  **Then** the message is moved to `.decree/inbox/dead/`

### Retry and Dead-Letter

- **Given** a routine fails on attempt 1 of 3 (not the last retry)
  **When** the failure is detected
  **Then** partial changes are captured in `changes.diff`
  **And** changes are left in place (no revert)
  **And** the routine is re-executed so the AI can learn from its mistakes

- **Given** a routine fails on attempt 3 of 3 (the final retry)
  **When** the failure is detected
  **Then** the working tree is reverted to the original checkpoint
  (pre-first-attempt state)
  **And** a `failure-context.md` is written summarizing all prior attempt
  errors
  **And** the routine is re-executed with a clean slate

- **Given** all retries are exhausted
  **When** the final attempt also fails
  **Then** the working tree is reverted to the original checkpoint
  **And** the message is moved to `.decree/inbox/dead/`
  **And** the run directory preserves all attempt artifacts

- **Given** `decree process` is running and a spec fails all retries
  **When** the message is dead-lettered
  **Then** `decree process` continues to the next unprocessed spec

- **Given** a message is in `.decree/inbox/dead/`
  **When** the user moves it back to `.decree/inbox/`
  **Then** it is picked up and processed again on the next run
