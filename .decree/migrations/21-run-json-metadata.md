---
routine: rust-develop
---

# 21: run.json per-run metadata

## Overview

Write a `run.json` file to the run directory at the end of processing so
each run has a machine-readable record of what happened.

## Requirements

Write a `run.json` file to the run directory at the end of processing (after
all retries, whether success or failure). Contents:

```json
{
  "message_id": "D0001-1432-fix-errors-0",
  "routine": "develop",
  "trigger": "inbox",
  "migration": "01-add-auth.md",
  "attempts": 2,
  "exit_code": 0,
  "start": "2026-04-28T14:30:00",
  "end": "2026-04-28T14:30:45",
  "duration_s": 45
}
```

Fields:
- `message_id` — full message ID
- `routine` — routine name used
- `trigger` — source of the run (placeholder value for now; will be wired up
  in the DECREE_TRIGGER migration)
- `migration` — migration filename if present, otherwise omitted
- `attempts` — total attempts made (1 on first-try success)
- `exit_code` — exit code of the final (or only) attempt; 0 on success
- `start` — ISO 8601 timestamp of first attempt start
- `end` — ISO 8601 timestamp of last attempt end
- `duration_s` — wall-clock seconds from start to end

## Files to Modify

- `src/commands/process.rs` — write `run.json` to run directory after all
  retries complete (success or failure)

## Acceptance Criteria

- **Given** a message that succeeds on the first attempt
  **When** processing completes
  **Then** `run.json` exists in the run directory with `exit_code: 0`,
  `attempts: 1`, and non-empty `start`, `end`, `duration_s` values

- **Given** a message that fails twice and succeeds on the third attempt
  **When** processing completes
  **Then** `run.json` has `exit_code: 0` and `attempts: 3`

- **Given** a message that exhausts all retries and is dead-lettered
  **When** processing completes
  **Then** `run.json` exists with `exit_code` equal to the last attempt's
  exit code and `attempts` equal to `max_retries`

- **Given** a message with a `migration` field
  **When** processing completes
  **Then** `run.json` includes the `migration` key
