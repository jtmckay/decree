# 10: Lifecycle Hooks

## Overview

Decree supports lifecycle hooks defined in `config.yml`. Hooks are
routines that run at specific points during processing. They follow
the same format as regular routines (shell scripts with pre-checks)
and live in `.decree/routines/`.

## Hook Types

```yaml
hooks:
  beforeAll: "setup"           # Runs once before any processing starts
  afterAll: "teardown"         # Runs once after all processing completes
  beforeEach: "pre-flight"     # Runs before each message is processed
  afterEach: "post-flight"     # Runs after each message is processed
```

### `beforeAll`

Runs once at the start of `decree process` or `decree daemon` before
any messages are processed. Use cases:
- Set up test environment
- Pull latest dependencies
- Verify system state

### `afterAll`

Runs once after all processing completes normally (all migrations
processed and inbox drained). **Does not run** on SIGINT/SIGTERM
shutdown of the daemon — the process follows a non-destructive path
that is recoverable without teardown. Use cases:
- Run final test suite
- Generate reports
- Clean up temporary resources

### `beforeEach`

Runs before each individual message is processed, after the run
directory is created but before the routine executes. Receives
`DECREE_ATTEMPT` to support retry-aware behavior. Use cases:
- Create per-message snapshots (e.g. git stash baseline)
- Reset test state
- Log processing start

### `afterEach`

Runs after each individual message is processed, regardless of
success or failure. Receives `DECREE_ROUTINE_EXIT_CODE` and
`DECREE_ATTEMPT` to support retry-aware behavior. Use cases:
- Checkpoint routine changes (e.g. git stash)
- Collect metrics
- Handle exhaustion recovery

## Hook Execution

Hooks are executed as regular routines with the same parameter
injection system. They receive the standard env vars:

- `message_file`, `message_id`, `message_dir`, `chain`, `seq`
  — for `beforeEach` and `afterEach` (scoped to current message)
- For `beforeAll` and `afterAll`, message-specific vars are empty

Additional env vars for hooks:

| Variable | Description |
|---|---|
| `DECREE_HOOK` | Hook type: `beforeAll`, `afterAll`, `beforeEach`, `afterEach` |
| `DECREE_ATTEMPT` | Current attempt number, 1-indexed (beforeEach/afterEach only) |
| `DECREE_MAX_RETRIES` | Configured max retries from config (beforeEach/afterEach only) |
| `DECREE_ROUTINE_EXIT_CODE` | Exit code of the routine (afterEach only) |

These variables allow hooks to implement retry-aware strategies (e.g.
git stash hooks deciding whether to revert based on attempt number)
without decree's core needing any VCS awareness.

## Hook Failure Behavior

- **`beforeAll` failure**: abort processing entirely, exit non-zero
- **`afterAll` failure**: log warning, exit with hook's exit code
- **`beforeEach` failure**: skip the current message, dead-letter it,
  continue to next
- **`afterEach` failure**: log warning, continue processing

Hook failures do not trigger the retry system — they fail once and
the failure behavior above applies.

## Empty Hooks

When a hook value is empty string `""` or the key is absent, no hook
runs for that phase. This is the default.

## Hook Pre-Checks

Hooks are regular routines and should include pre-check sections.
`decree verify` includes hooks in its pre-check scan.

## Default Hook Templates

Decree ships git stash hooks as the default `beforeEach`/`afterEach`
pair (see migration 08). These create a stash baseline before each
routine and checkpoint the routine's changes afterward. Users can
replace these with custom hooks or leave them empty.

## Execution Order

```
beforeAll
  for each message:
    beforeEach  (DECREE_ATTEMPT=N)
    routine execution
    afterEach   (DECREE_ATTEMPT=N, DECREE_ROUTINE_EXIT_CODE=X)
    if failure and retries remain:
      beforeEach  (DECREE_ATTEMPT=N+1)
      routine execution
      afterEach   (DECREE_ATTEMPT=N+1, DECREE_ROUTINE_EXIT_CODE=X)
afterAll (only on normal completion, not SIGINT)
```

## Acceptance Criteria

- [ ] `beforeAll` hook runs once before any processing starts
- [ ] `afterAll` hook runs once after all processing completes normally
- [ ] `afterAll` does NOT run on SIGINT/SIGTERM shutdown
- [ ] `beforeEach` hook runs before each message's routine (and each retry)
- [ ] `afterEach` hook runs after each message's routine (and each retry)
- [ ] Hook routines receive standard env vars
- [ ] `DECREE_HOOK` env var is set to the hook type
- [ ] `DECREE_ATTEMPT` and `DECREE_MAX_RETRIES` are set for beforeEach/afterEach
- [ ] `DECREE_ROUTINE_EXIT_CODE` is set for afterEach
- [ ] `beforeAll` failure aborts all processing
- [ ] `afterAll` failure logs warning, exits with hook's code
- [ ] `beforeEach` failure skips and dead-letters the message
- [ ] `afterEach` failure logs warning, continues
- [ ] Empty/absent hook values mean no hook runs
- [ ] Hooks are included in `decree verify` pre-check scan
- [ ] Decree core has zero VCS awareness — hooks handle all state management
