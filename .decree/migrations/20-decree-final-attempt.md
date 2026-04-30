---
routine: rust-develop
---

# 20: DECREE_FINAL_ATTEMPT env var

## Overview

Expose whether the current retry attempt is the last one via a
`DECREE_FINAL_ATTEMPT` env var so routines and hooks can behave differently
on the final try.

## Requirements

Add a `final_attempt: bool` field to `HookContext`. Set it to `true` when
the current attempt equals `max_retries` (the last allowed attempt). Pass
it as the env var `DECREE_FINAL_ATTEMPT=true` in `afterEach` hooks.

The variable is only set when `true`; it is absent from the environment on
non-final attempts.

## Files to Modify

- `src/hooks.rs` — `HookContext` gains `final_attempt: bool`; pass
  `DECREE_FINAL_ATTEMPT=true` when set in `run_hook_with_config`
- `src/commands/process.rs` — set `final_attempt` on the hook context when
  `attempt == max_retries`

## Acceptance Criteria

- **Given** `max_retries: 3` and a routine that always fails
  **When** attempt 1 or 2 runs
  **Then** `DECREE_FINAL_ATTEMPT` is NOT set in the `afterEach` environment

- **Given** `max_retries: 3` and a routine that always fails
  **When** attempt 3 (the final attempt) runs
  **Then** `DECREE_FINAL_ATTEMPT=true` is set in the `afterEach` environment

- **Given** `max_retries: 1` and a routine that succeeds on attempt 1
  **When** `afterEach` runs
  **Then** `DECREE_FINAL_ATTEMPT=true` is set (only attempt is always final)
