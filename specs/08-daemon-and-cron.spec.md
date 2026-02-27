---
routine: rust-develop
---

# 08: Daemon and Cron

## Overview

`decree daemon` continuously monitors the inbox and cron directory,
processing messages as they appear. Cron files define scheduled tasks
that fire on a cron expression. The daemon evaluates cron files on each
poll, copies due jobs into the inbox as new messages, and processes them
through the standard pipeline (spec 07). Every acceptance criterion
asserts end-to-end processing — not just detection.

## Requirements

### `decree daemon`

```
decree daemon                    # poll every 2 seconds (default)
decree daemon --interval 5       # poll every 5 seconds
```

### Daemon Polling Loop

1. Check `.decree/cron/` for due cron jobs (see "Cron Directory" below)
2. Copy due jobs into `.decree/inbox/` as new messages
3. Check `.decree/inbox/` for pending message files
4. If messages found: process them (depth-first within chains, see spec 07)
5. Sleep for the interval
6. Go to step 1
7. Exit on SIGINT/SIGTERM (graceful shutdown after current message completes)

### Cron Directory

Cron files are `.md` files in `.decree/cron/` with a `cron` property in
their YAML frontmatter. The `cron` value is a standard cron expression
(5 fields: minute, hour, day-of-month, month, day-of-week).

#### Cron file format

```yaml
---
cron: "0 * * * *"
routine: develop
---
Run the hourly maintenance task.
```

Fields:
- `cron` (required) — standard 5-field cron expression
- `routine` (optional) — which routine to use (follows normal selection chain if omitted)
- Any other frontmatter fields are passed through to the inbox message

The markdown body after frontmatter is the task description, copied into
the inbox message body.

### Daemon Cron Evaluation

On each poll iteration, before checking the inbox:

1. Scan `.decree/cron/` for `*.md` files with a `cron` frontmatter field
2. For each cron file, evaluate whether the cron expression is due
   (matches the current minute). Track the last fire time per cron file
   in memory to avoid duplicate firings within the same minute.
3. For due jobs, create a new inbox message file:
   - Generate a new chain ID
   - Name the file `<chain>-0.md` (standard inbox naming)
   - Set `seq: 0`, `type: task`
   - Copy `routine` and any custom frontmatter fields (strip `cron`)
   - Copy the markdown body as the message body
4. The resulting inbox message is a normal message — processed by the
   standard message processing loop (spec 07)

Cron files are **not consumed** — they stay in `.decree/cron/` and fire
repeatedly on schedule. The daemon only tracks fire times in memory;
restarting the daemon resets tracking (a job may fire once on restart if
its schedule matches the current minute).

### Cron-to-Inbox Message Creation

When a cron file fires, the daemon creates an inbox message that passes
through the full processing pipeline:

1. Generate a new chain ID
2. Create `.decree/inbox/<chain>-0.md`
3. Write YAML frontmatter:
   - `seq: 0`
   - `type: task`
   - `routine` from cron file (if present)
   - All custom frontmatter fields from cron file
   - **Strip the `cron` field** — it is not copied to the inbox message
4. Copy the markdown body from the cron file
5. The message is then normalized (spec 05), checkpointed (spec 04),
   executed (spec 06/07), and dispositioned like any other message

## Acceptance Criteria

- **Given** `decree daemon` is running and a message file appears in `.decree/inbox/`
  **When** the daemon polls and detects the message
  **Then** a run directory `.decree/runs/<chain>-<seq>/` is created
  **And** the routine is executed (artifacts exist in the run directory)
  **And** the message is moved to `.decree/inbox/done/`

- **Given** `decree daemon` is running and a cron file fires
  **When** the daemon creates an inbox message from the cron file
  **Then** the inbox message is processed in the same or next poll cycle
  **And** a run directory is created with execution artifacts
  **And** the message is moved to `.decree/inbox/done/`

- **Given** a cron file fires and creates an inbox message
  **When** the message is processed
  **Then** the full pipeline executes: normalize (spec 05), checkpoint (spec 04),
  execute routine (spec 06), generate diff (spec 04), disposition to `done/`

- **Given** `decree daemon` is running and SIGINT is received
  **When** a message is currently being processed
  **Then** the current message completes with all artifacts (run directory,
  routine.log or output.ipynb, changes.diff) before the daemon exits

- **Given** a cron file with `cron: "* * * * *"` has already fired in the current minute
  **When** the daemon polls again within the same minute
  **Then** the cron file does not fire again (no duplicate inbox message)

- **Given** a cron file has `routine: develop` and custom frontmatter fields
  **When** the daemon copies it to the inbox
  **Then** the `routine` and custom fields are preserved in the inbox message
  **And** the `cron` field is stripped (not present in the inbox message)

- **Given** a cron file fires and its inbox message is processed successfully
  **When** the next matching cron minute arrives
  **Then** the cron file fires again (cron files are never consumed)

- **Given** `decree daemon` is restarted
  **When** the current minute matches a cron expression
  **Then** the cron file fires (fire time tracking resets on restart)

- **Given** a routine spawns a follow-up message during daemon processing
  **When** the follow-up appears in the inbox
  **Then** it is processed depth-first within the same chain before other messages

- **Given** a message exhausts all retries during daemon processing
  **When** the message is dead-lettered to `.decree/inbox/dead/`
  **Then** the daemon continues polling and processing other messages
