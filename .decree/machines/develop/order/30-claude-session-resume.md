---
routine: rust-develop
---

# 30: Resume Claude session on token-exhaustion retry

## Overview

After a token-exhaustion wait (migration 29), pass the previous Claude
session ID to the retried routine so it can resume the conversation rather
than starting fresh.

## Requirements

### Capturing the session ID

The Claude CLI prints the active session ID to stdout in the format:

```
Session ID: <id>
```

or as part of its JSON output. After each routine attempt, scan the run
log for this pattern and, if found, persist the session ID to the run
directory as `session_id.txt`.

The scan pattern (case-insensitive):

```
[Ss]ession(?:\s+[Ii][Dd])?:\s*([a-zA-Z0-9_-]+)
```

If no session ID is found in the log, `session_id.txt` is not written.

### Passing the session ID on retry

When `process_single_message` is about to retry a message after a token-
exhaustion wait (migration 29), it checks the run directory of the previous
attempt for `session_id.txt`. If the file exists and is non-empty, it reads
the ID and sets `DECREE_PREVIOUS_SESSION_ID=<id>` in the routine's
environment for the retry attempt.

The env var is only set for the first retry after a token-exhaustion wait.
Subsequent normal retries (if the resumed attempt also fails) do not carry
the session ID forward.

### Routine opt-in

Routines may use the env var to resume the session, for example:

```bash
resume_flag=""
if [ -n "${DECREE_PREVIOUS_SESSION_ID:-}" ]; then
  resume_flag="--resume $DECREE_PREVIOUS_SESSION_ID"
fi
claude $resume_flag -p "$prompt"
```

Setting the env var is the only change Decree makes; whether to use it is
left to the routine author.

### Update default routine templates

Update `src/templates/develop.sh` and `src/templates/rust-develop.sh` to
use `DECREE_PREVIOUS_SESSION_ID` when invoking Claude, following the
opt-in pattern above.

## Files to Modify

- `src/commands/process.rs` — scan log for session ID after each attempt;
  write `session_id.txt` to run dir; read it and set env var before
  token-exhaustion retry
- `src/templates/develop.sh` — use `DECREE_PREVIOUS_SESSION_ID` when set
- `src/templates/rust-develop.sh` — use `DECREE_PREVIOUS_SESSION_ID` when set

## Acceptance Criteria

- **Given** a routine attempt whose log contains `"Session ID: abc123"`
  **When** the attempt completes
  **Then** `session_id.txt` containing `abc123` is written to the run
  directory

- **Given** a token-exhaustion wait that completes and a `session_id.txt`
  from the prior attempt
  **When** the retry attempt starts
  **Then** `DECREE_PREVIOUS_SESSION_ID=abc123` is set in the routine
  environment

- **Given** a routine attempt whose log contains no session ID
  **When** the attempt completes
  **Then** no `session_id.txt` is written and `DECREE_PREVIOUS_SESSION_ID`
  is not set on any retry

- **Given** a token-exhaustion retry that fails again for a non-token reason
  **When** the second retry attempt starts (normal retry, not token retry)
  **Then** `DECREE_PREVIOUS_SESSION_ID` is NOT set

- **Given** the updated `develop.sh` template is installed via `decree init`
  **When** `DECREE_PREVIOUS_SESSION_ID` is set
  **Then** Claude is invoked with the `--resume` flag and the session ID
