---
routine: rust-develop
---

# 22: Per-routine max_retries and timeout_s

## Overview

Allow individual routines to override the global `max_retries` and add a
`timeout_s` cap so long-running routines can be killed automatically.

## Requirements

Extend `RoutineEntry` with two optional overrides:

```yaml
routines:
  gmail-sync:
    enabled: true
    max_retries: 5
  actual-budget:
    enabled: true
    timeout_s: 60
```

**max_retries**: when set, overrides the global `max_retries` for messages
targeting that routine. The per-routine value is used in the retry loop and
for `DECREE_MAX_RETRIES` / `DECREE_FINAL_ATTEMPT` calculations.

**timeout_s**: when set, the routine process is killed with SIGTERM after
this many seconds and the attempt is treated as exit code 1 (failure). If
the routine times out on the final attempt, the message is dead-lettered.

Timeouts do not affect hooks. If `timeout_s` is not set, behaviour is
unchanged.

## Files to Modify

- `src/config.rs` — `RoutineEntry` gains `max_retries: Option<u32>` and
  `timeout_s: Option<u32>`
- `src/commands/process.rs` — use per-routine `max_retries` when set;
  enforce `timeout_s` by killing the process after the specified duration

## Acceptance Criteria

- **Given** a routine entry `gmail-sync: {enabled: true, max_retries: 5}`
  and global `max_retries: 3`
  **When** a message targeting `gmail-sync` is processed
  **Then** the retry loop runs up to 5 attempts (not 3)

- **Given** a routine entry with no `max_retries`
  **When** a message targeting that routine is processed
  **Then** the global `max_retries` is used

- **Given** a routine entry `actual-budget: {enabled: true, timeout_s: 1}`
  and a routine script that sleeps for 10 seconds
  **When** the routine runs
  **Then** the process is killed after ~1 second and the attempt is recorded
  as exit code 1 (failure)

- **Given** a routine entry with no `timeout_s`
  **When** the routine runs
  **Then** no timeout is applied (unchanged behaviour)
