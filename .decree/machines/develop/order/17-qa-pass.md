---
routine: rust-develop
---

# 17: QA Pass — Hook Logging, Routine Isolation, and Router Fix

## Overview

Three fixes discovered during black-box testing of v0.3.0.

1. **Hook output not captured in logs.** Pre-hook (beforeEach, beforeAll)
   and post-hook (afterEach, afterAll) stdout/stderr is discarded. It
   should be appended to the routine's log file in the run directory so
   operators can diagnose hook failures from `decree log`.

2. **`decree routine <name>` drains the inbox.** Running a specific
   routine should only execute the single message being created — it
   should not pick up unrelated inbox messages. Today it calls
   `process::run()` which processes all pending migrations and drains
   the inbox.

3. **AI router not invoked during processing.** When a migration or
   inbox message has no `routine` frontmatter field, `process.rs` calls
   `normalize()` with `ai_router=None`, so the router is never used.
   It silently falls back to `default_routine`. The router should be
   invoked with only the enabled routines from config.

## Requirements

### 1. Capture hook output in routine logs

In `src/hooks.rs`, change hook execution so stdout and stderr are
captured and returned to the caller.

In `src/commands/process.rs`, append hook output to the current
message's log file. The format should match existing log conventions:

```
[decree] hook beforeEach start 2026-03-13T17:00:00
<hook stdout/stderr here>
[decree] hook beforeEach end 2026-03-13T17:00:01
```

For beforeAll/afterAll hooks (which run outside any single message
context), write to a top-level log or to stderr — the important thing
is that the output is not silently discarded.

If a hook produces no output, omit the hook log block entirely.

Hooks that fail already produce a warning; continue doing so, but also
include the captured output in the log file.

### 2. Isolate `decree routine <name>` from inbox

In `src/commands/routine.rs`, after creating the inbox message for the
requested routine, do NOT call the full `process::run()`. Instead,
process only the single message that was just created.

This means:
- No beforeAll / afterAll hooks (this is a one-off run, not a batch)
- No migration scanning
- No inbox draining beyond the single message
- beforeEach / afterEach hooks still run for the single message
- Retry logic still applies
- The run directory and logs are still created as normal

Extract or expose enough of the processing logic from `process.rs` so
that `routine.rs` can process a single message without triggering the
full pipeline.

### 3. Invoke AI router for routine selection

In `src/commands/process.rs`, when calling `normalize()`, pass the AI
router callback instead of `None`. Build the callback using
`config.commands.ai_router` — the same pattern already used in the
router prompt template (`router.md`).

The router prompt must list only enabled routines (both project-local
and shared). The `list_routines()` function already filters by
enabled status, so use it to build the prompt.

If the AI router command is not configured or fails, fall back to
`config.default_routine` as today. Do not error out — the router is
best-effort.

## Files to Modify

- `src/hooks.rs` — return captured stdout/stderr from hook execution
- `src/commands/process.rs` — write hook output to log files; pass AI
  router to normalize(); extract single-message processing
- `src/commands/routine.rs` — use single-message processing instead of
  full `process::run()`
- `src/message.rs` — no changes expected, but verify `normalize()`
  correctly uses the router callback when provided

## Acceptance Criteria

### Hook output capture

- **Given** a beforeEach hook that prints "BASELINE SAVED" to stdout
  **When** a migration is processed
  **Then** the run directory's log contains "[decree] hook beforeEach"
  and "BASELINE SAVED"

- **Given** an afterEach hook that prints to stderr
  **When** a migration is processed
  **Then** the hook's stderr appears in the routine log

- **Given** a hook that produces no output
  **When** a migration is processed
  **Then** no hook log block is written (no empty markers)

- **Given** a hook that fails
  **When** the failure is logged
  **Then** both the warning and the hook's captured output appear in
  the log

### Routine isolation

- **Given** an inbox message `A.md` already exists
  **When** the user runs `decree routine develop` and creates message B
  **Then** only message B is processed; `A.md` remains in the inbox

- **Given** pending migrations exist
  **When** the user runs `decree routine develop`
  **Then** no migrations are processed; they remain pending

- **Given** `decree routine develop` is run
  **When** the routine fails and retries are exhausted
  **Then** the message is dead-lettered as normal

- **Given** `decree routine develop` is run
  **When** beforeEach/afterEach hooks are configured
  **Then** the hooks run for the single message

- **Given** `decree routine develop` is run
  **When** beforeAll/afterAll hooks are configured
  **Then** beforeAll/afterAll do NOT run (single-message mode)

### AI router invocation

- **Given** a migration with no `routine` field and `ai_router`
  configured
  **When** the migration is processed
  **Then** the AI router is invoked to select a routine

- **Given** the AI router returns "rust-develop"
  **When** the message is normalized
  **Then** the message's routine is set to "rust-develop"

- **Given** the AI router is not configured (empty `ai_router`)
  **When** a migration with no routine is processed
  **Then** `default_routine` from config is used (no error)

- **Given** the AI router command fails (non-zero exit)
  **When** a migration with no routine is processed
  **Then** `default_routine` is used as fallback and a warning is
  printed

- **Given** enabled routines: develop, rust-develop; disabled: deploy
  **When** the AI router prompt is built
  **Then** the prompt lists develop and rust-develop but NOT deploy
