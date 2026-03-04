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

Runs once after all processing completes (all migrations processed and
inbox drained). Use cases:
- Run final test suite
- Generate reports
- Clean up temporary resources

### `beforeEach`

Runs before each individual message is processed, after the run
directory is created but before the routine executes. Use cases:
- Reset test state
- Create per-message snapshots
- Log processing start

### `afterEach`

Runs after each individual message is processed, regardless of
success or failure. Use cases:
- Collect metrics
- Post-process output
- Notify on completion

## Hook Execution

Hooks are executed as regular routines with the same parameter
injection system. They receive the standard env vars:

- `message_file`, `message_id`, `message_dir`, `chain`, `seq`,
  `input_file` (if set) — for `beforeEach` and `afterEach` (scoped to current message)
- For `beforeAll` and `afterAll`, message-specific vars are empty

Additional env var for hooks:
- `DECREE_HOOK` — set to the hook type (`beforeAll`, `afterAll`,
  `beforeEach`, `afterEach`)

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
pair (see migration 08). These create a git baseline before each
routine and stash only the routine's changes afterward. Users can
replace these with custom hooks or leave them empty.

## Execution Order

```
beforeAll
  for each message:
    beforeEach
    routine execution (with retries)
    afterEach
afterAll
```

## Acceptance Criteria

- [ ] `beforeAll` hook runs once before any processing starts
- [ ] `afterAll` hook runs once after all processing completes
- [ ] `beforeEach` hook runs before each message's routine
- [ ] `afterEach` hook runs after each message's routine
- [ ] Hook routines receive standard env vars
- [ ] `DECREE_HOOK` env var is set to the hook type
- [ ] `beforeAll` failure aborts all processing
- [ ] `afterAll` failure logs warning, exits with hook's code
- [ ] `beforeEach` failure skips and dead-letters the message
- [ ] `afterEach` failure logs warning, continues
- [ ] Empty/absent hook values mean no hook runs
- [ ] Hooks are included in `decree verify` pre-check scan
