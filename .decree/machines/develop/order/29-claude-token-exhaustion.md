---
routine: rust-develop
---

# 29: Claude token exhaustion: detect, wait, and retry

## Overview

When Claude's usage limit is hit, the routine exits non-zero and prints a
message containing the reset time. Detect this specific condition, compute
how long to wait, sleep until the reset, un-mark the migration from
processed so it can be retried, and restart the migration loop from the
failed migration.

## Requirements

### Detection

After a routine exits non-zero, scan the combined stdout+stderr log for the
Claude usage-limit pattern. The pattern matches any log content that contains
both of the following (case-insensitive):

- the phrase `usage limit` (e.g. "Claude AI usage limit reached", "usage
  limit reached")
- the phrase `reset` (e.g. "Limits reset at 10:00 PM", "resets at 5:00 AM")

If the pattern matches, treat the failure as a token-exhaustion event rather
than a normal failure.

### Reset-time extraction

Attempt to extract an absolute reset time from the log using a regex that
matches common Claude CLI formats, for example:

```
[Ll]imit[s]? reset[s]? (?:at )?(\d{1,2}:\d{2}\s*(?:AM|PM)(?:\s+\w+)?)
```

If a time is found, parse it as a time-of-day in the local timezone. If the
parsed time is in the past (i.e., the reset has not happened yet today but
the clock has not ticked past it), add 24 hours. If parsing fails or no
time is found, fall back to a default wait of **1 hour**.

### Wait behaviour

Print a message to stderr before sleeping:

```
[Claude token limit] Usage limit reached. Waiting until HH:MM (Xm Ys) to retry.
```

Sleep until the computed reset time. The process must remain alive during
the wait; it must not exit. Honor SIGINT during the wait: if the user presses
Ctrl+C during the sleep, exit immediately with code 130.

### Retry

After the wait completes:

1. Remove the migration from `.decree/processed.md` so it is treated as
   unprocessed again.
2. Remove the dead-lettered inbox message (if it was moved to `inbox/dead/`)
   so there is no stale copy.
3. Print:
   ```
   [Claude token limit] Retrying migration: <migration_filename>
   ```
4. Re-enter the migration loop from the failed migration (the outer loop
   will pick it up naturally once it is no longer in `processed.md`).

### Interaction with stop-on-error (migration 28)

Token exhaustion must be detected and handled **before** `DrainResult`
propagates the dead-letter flag to the outer migration loop. When token
exhaustion is detected, the dead-letter flag must NOT be set — the caller
sees the migration as still pending, not as a hard failure.

### New error variant

Add `DecreeError::TokenLimitExhausted { reset_at: Option<chrono::DateTime<chrono::Local>> }`
to `src/error.rs`. Raise this variant from `execute_routine` (or from the
retry loop in `process_single_message`) when the usage-limit pattern is
detected. The outer logic in `process_single_message` catches this variant,
performs the wait, and signals to the caller that the message should be
retried rather than dead-lettered.

## Files to Modify

- `src/error.rs` — add `TokenLimitExhausted` variant
- `src/commands/process.rs` — detect token exhaustion after non-zero exit;
  parse reset time; sleep with SIGINT awareness; un-mark migration from
  processed; remove dead-letter copy; re-enter the loop

## Acceptance Criteria

- **Given** a routine whose log output contains `"Claude AI usage limit reached. Limits reset at 10:00 PM"`
  **When** the routine exits non-zero
  **Then** the process does not immediately dead-letter the message; instead
  it prints a waiting message and sleeps until 10:00 PM

- **Given** a token-exhaustion wait that completes
  **When** the sleep finishes
  **Then** the failed migration is removed from `processed.md`, any
  dead-lettered copy is removed, and the migration loop restarts the
  migration

- **Given** a routine log that contains `"usage limit"` and `"reset"` but no
  parseable time
  **When** token exhaustion is detected
  **Then** the process waits 1 hour (the default fallback)

- **Given** the user presses Ctrl+C during the token-exhaustion sleep
  **When** SIGINT is received
  **Then** the process exits immediately with code 130

- **Given** a routine that exits non-zero with a log that does NOT match the
  usage-limit pattern
  **When** the routine fails
  **Then** normal retry and dead-letter behaviour is unchanged

- **Given** the migration is retried after a token-exhaustion wait and
  succeeds
  **When** processing continues
  **Then** the migration is marked processed normally and the next migration
  starts
