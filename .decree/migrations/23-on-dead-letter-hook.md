---
routine: rust-develop
---

# 23: onDeadLetter hook

## Overview

Add a new `onDeadLetter` lifecycle hook that fires exactly once when a
message is moved to `inbox/dead/` after exhausting all retries.

## Requirements

Add a new `onDeadLetter` hook phase to `HooksConfig`:

```yaml
hooks:
  onDeadLetter: /work/.decree/hooks/on-dead-letter.sh
```

The hook fires exactly once when a message is moved to `inbox/dead/`, after
all retries are exhausted. It does NOT fire when `beforeEach` fails and the
message is dead-lettered immediately.

Available env vars: all standard message vars (`message_file`, `message_id`,
`message_dir`, `chain`, `seq`) plus `DECREE_ATTEMPT` (= effective
max_retries for this routine), `DECREE_MAX_RETRIES` (= same),
`DECREE_ROUTINE_EXIT_CODE` (exit code of last attempt), and
`DECREE_TRIGGER`.

The hook runs after the message has already been moved to `inbox/dead/`, so
`message_file` points to the dead-letter path.

A failure of `onDeadLetter` is logged as a warning but does not affect
processing.

`HookType` gains an `OnDeadLetter` variant with string representation
`"onDeadLetter"`. Add `on_dead_letter` to `configured_hook_names`.

## Files to Modify

- `src/config.rs` — `HooksConfig` gains `on_dead_letter: Option<String>`
- `src/hooks.rs` — `HookType` gains `OnDeadLetter` variant; add
  `on_dead_letter` to `configured_hook_names`; `run_hook_with_config`
  handles the new variant
- `src/commands/process.rs` — call `onDeadLetter` hook after moving message
  to `inbox/dead/` when retries are exhausted (not on `beforeEach` failure)

## Acceptance Criteria

- **Given** an `onDeadLetter` hook configured and a message that exhausts
  all retries
  **When** the message is moved to `inbox/dead/`
  **Then** the hook fires exactly once with `message_id`, `DECREE_ATTEMPT`,
  `DECREE_MAX_RETRIES`, `DECREE_ROUTINE_EXIT_CODE`, and `DECREE_TRIGGER`
  all set correctly

- **Given** a `beforeEach` hook failure that causes immediate dead-lettering
  **When** the message is moved to `inbox/dead/`
  **Then** `onDeadLetter` does NOT fire

- **Given** an `onDeadLetter` hook that exits non-zero
  **When** it fires
  **Then** a warning is printed but processing continues normally

- **Given** no `onDeadLetter` hook configured
  **When** a message is dead-lettered
  **Then** no error occurs (unchanged behaviour)
