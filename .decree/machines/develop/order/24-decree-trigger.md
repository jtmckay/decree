---
routine: rust-develop
---

# 24: DECREE_TRIGGER env var

## Overview

Track how a run was initiated — from a migration, a cron job, or a chain
follow-up — and expose it as `DECREE_TRIGGER` in routines and hooks.

## Requirements

Track how a run was initiated. Add a `trigger: Option<String>` field to
`InboxMessage` as a known field (not a custom field). Set it at message
creation time:

| Trigger value    | Set when                                                      |
|------------------|---------------------------------------------------------------|
| `cron:<stem>`    | Cron daemon fires a cron file with stem `<stem>`              |
| `inbox`          | Message was manually dropped into inbox or came from a migration |
| `chain`          | Message was promoted from outbox as a follow-up               |

(webhook is reserved for future use; do not implement it in this migration.)

Pass `DECREE_TRIGGER` as an env var in:
- Routine execution (`execute_routine`)
- All hook phases (beforeAll, afterAll, beforeEach, afterEach, onDeadLetter)

Include `trigger` in `run.json`.

The field `trigger` is added to `KNOWN_FIELDS` in `message.rs` so it is not
treated as a custom field.

For messages that already exist in the inbox without a trigger field,
normalize to `"inbox"` as the default.

**Where to set trigger:**
- `process.rs` — migration → inbox message: set `trigger = "inbox"`
- `process.rs` — `collect_outbox`: set `trigger = "chain"` on follow-up messages
- `daemon.rs` (or wherever cron messages are created): set `trigger = "cron:<stem>"`
- `InboxMessage::normalize()` — default trigger to `"inbox"` when absent

## Files to Modify

- `src/message.rs` — add `trigger` to `KNOWN_FIELDS`; serialize/deserialize
  `trigger` in `InboxMessage`; default trigger to `"inbox"` in `normalize()`
- `src/commands/process.rs` — set `trigger = "inbox"` for migration messages;
  set `trigger = "chain"` in `collect_outbox`; pass `DECREE_TRIGGER` to
  routine env and hook context; include `trigger` in `run.json`
- `src/hooks.rs` — `HookContext` gains `trigger: String`; pass
  `DECREE_TRIGGER` in `run_hook_with_config`
- `src/commands/daemon.rs` — set `trigger = "cron:<stem>"` when creating
  messages from cron files

## Acceptance Criteria

- **Given** a message created from a migration (via `decree process`)
  **When** the routine executes
  **Then** `DECREE_TRIGGER=inbox` is set in the routine environment

- **Given** a cron file `gmail-sync.md` that fires and creates an inbox
  message
  **When** the routine executes
  **Then** `DECREE_TRIGGER=cron:gmail-sync` is set in the routine environment

- **Given** a follow-up message promoted from the outbox
  **When** the routine executes
  **Then** `DECREE_TRIGGER=chain` is set in the routine environment

- **Given** a message with any trigger
  **When** `beforeEach`, `afterEach`, or `onDeadLetter` hooks run
  **Then** `DECREE_TRIGGER` is set to the same value as in the routine
  environment

- **Given** an existing inbox message without a `trigger` field
  **When** `normalize()` runs
  **Then** trigger defaults to `"inbox"`

- **Given** `run.json` is written after processing
  **Then** it includes the `trigger` value
