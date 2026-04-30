---
routine: rust-develop
---

# 19: Inbox filename normalization

## Overview

When `normalize()` generates a new chain ID for a message with no chain/seq
in its filename and no chain in frontmatter, it currently falls back to
`"message"` as the name component. Use the inbox file's own stem instead,
and rename the file after normalization.

## Requirements

When `normalize()` generates a new chain ID for a message that has no
chain/seq in its filename and no chain in frontmatter, it currently falls
back to `"message"` as the name component, producing `D0001-1432-message-0`.

Change the fallback to use the inbox file's own stem. A file named
`fix-errors.md` should produce chain `D0001-1432-fix-errors` and full ID
`D0001-1432-fix-errors-0`.

After normalization generates a new ID, the inbox file must be renamed from
its original name to the new message ID filename. The `process_single_message`
function must track the active filename and pass the updated name to
subsequent operations (beforeEach, routine execution, dead-lettering).

The existing `migration`-field fallback (use migration stem when present)
continues to take priority over the filename stem.

## Files to Modify

- `src/message.rs` — inbox filename stem fallback in `normalize()`
- `src/commands/process.rs` — rename inbox file after normalization; track
  active filename through subsequent operations

## Acceptance Criteria

- **Given** a file `fix-errors.md` dropped into inbox with no frontmatter
  **When** `normalize()` runs
  **Then** the chain is `D<NNNN>-HHmm-fix-errors`, the id is
  `D<NNNN>-HHmm-fix-errors-0`, and the inbox file is renamed to
  `D<NNNN>-HHmm-fix-errors-0.md`

- **Given** a message with `migration: 01-auth.md` and filename `random.md`
  **When** `normalize()` runs
  **Then** the chain uses `01-auth` as the name component (migration takes
  priority over filename stem)

- **Given** a message whose filename already matches the chain-seq pattern
  (e.g., `D0001-1432-foo-0.md`)
  **When** `normalize()` runs
  **Then** no rename occurs (filename is already correct)
