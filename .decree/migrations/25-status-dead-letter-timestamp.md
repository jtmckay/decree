---
routine: rust-develop
---

# 25: decree status dead-letter timestamp

## Overview

When dead-lettered messages exist, `decree status` should show the oldest
file's modification time so the operator knows how long messages have been
stuck.

## Requirements

`decree status` currently shows dead-letter count with no temporal context.
When at least one dead-letter exists, show the oldest file's modification
time:

```
  Dead-lettered: 3 messages  (oldest: 2026-04-26T02:19:34)
```

Use file modification time (`std::fs::metadata(...).modified()`). If the
directory does not exist or is empty, show the existing format unchanged.

## Files to Modify

- `src/commands/status.rs` — read mtime of files in `inbox/dead/` and
  display the oldest when at least one file exists

## Acceptance Criteria

- **Given** dead-letter files exist in `inbox/dead/`
  **When** `decree status` runs
  **Then** the output includes a line like
  `  Dead-lettered: 3 messages  (oldest: 2026-04-26T02:19:34)`

- **Given** no dead-letter files exist
  **When** `decree status` runs
  **Then** the dead-letter line shows the count only, with no timestamp
