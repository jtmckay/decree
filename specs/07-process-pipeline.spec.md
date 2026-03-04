# 07: Process and Pipeline

## Overview

`decree process` batch-processes all unprocessed migrations and then
drains the inbox of any spawned messages. This is the primary command
for executing work — no daemon required for basic use. The core message
processing loop handles lifecycle hooks (migration 10), routine
execution (migration 04), retry strategy, and dead-lettering.

## `decree process` — Batch Processing + Inbox Drain

```
decree process
decree              # bare `decree` is equivalent
```

`decree process`:
1. Run `beforeAll` hook (if configured)
2. Read `migrations/processed.md` to find unprocessed migrations
3. For the first unprocessed migration (alphabetically):
   - Generate a new chain ID
   - Create an inbox message (`type: spec`, `seq: 0`)
   - Process the message and any follow-up messages in its chain
   - On success: mark migration as processed in `processed.md`
4. Move to the next unprocessed migration (new chain, new chain ID)
5. Repeat until all migrations are processed
6. **Drain the inbox**: check `.decree/inbox/` for remaining messages
   and process them
7. Run `afterAll` hook (if configured)
8. Exit when inbox is empty

Each migration starts a new chain. If a migration fails all retries,
its message is dead-lettered and processing continues to the next.

## Message Processing

The core processing logic shared by `decree process` and `decree daemon`:

```
take next message from inbox
  (prefer messages from current chain — depth-first)
normalize message (fill missing fields — see migration 03)
create .decree/runs/<chain>-<seq>/
copy normalized message -> .decree/runs/<chain>-<seq>/message.md

1. Run beforeEach hook (if configured)
2. Determine routine (from normalized message frontmatter)
3. Resolve routine file (see migration 04)
4. Execute routine:
   bash .decree/routines/<routine>.sh 2>&1 | tee <msg-dir>/<log-file>
   - Log file: routine.log (attempt 1), routine-2.log (attempt 2), etc.
   - Env vars: message_file, message_id, message_dir,
     chain, seq, input_file (if set), plus custom variables
   - Working directory: project root
   - Output is streamed to the terminal in real-time AND logged
   - After execution: truncate log if it exceeds max_log_size
   - The routine runs as a child process in the same process group
   - Ctrl-C (SIGINT) kills the routine and any AI subprocess immediately

5. On SUCCESS:
   - Run afterEach hook
   - Move message to .decree/inbox/done/
   - If type=spec: append filename to processed.md
6. On FAILURE (see "Retry Strategy")
7. On EXHAUSTION (all retries failed):
   - If git hooks active: revert to baseline, undo baseline commit
   - Move message to .decree/inbox/dead/
   - Run afterEach hook
   - Continue to next message
8. Check inbox for new messages in this chain
   - If found and seq < max_depth: process depth-first
   - If seq >= max_depth: dead-letter with MaxDepthExceeded
```

## Retry Strategy

When a routine fails:

**Attempts 1 through N-1** (not the last retry):
- Leave changes in place — do not revert
- Re-execute the routine (AI can learn from its mistakes)

**Attempt N** (final retry, N = `max_retries`):
- If git stash hooks are active: revert to baseline commit
  (`git checkout . && git clean -fd`)
- Write `failure-context.md` to run dir summarizing prior attempt errors
- Re-execute with clean slate + failure context

**After all retries exhausted:**
- If git stash hooks active: revert to baseline, undo baseline commit
- Move message to `.decree/inbox/dead/`
- Run directory preserved for debugging

## Dead-Letter Directory

Messages moved to `.decree/inbox/dead/` when:
- All retries exhausted
- `MaxDepthExceeded` triggered
- Normalization fails (e.g. `RoutineNotFound`)

Users can re-queue by moving back to `.decree/inbox/`.

## Depth Limiting

Sequence number acts as depth counter:
- Root messages: `seq: 0`
- Each follow-up increments by 1
- When `seq >= max_depth` (default: 10): dead-letter with error

## Routine Execution and Signal Handling

### Real-Time Output

Routines are executed via `bash <routine>.sh 2>&1 | tee <msg-dir>/routine.log`.
This means:
- The AI tool's streaming output (reasoning, tool calls, progress) is
  visible in the terminal as it happens
- The same output is simultaneously written to `routine.log`
- The user can watch the AI work in real-time and review logs later

### Log Size Management

AI tools can produce enormous output (reasoning traces, file contents,
tool calls). A single `opencode run` invocation can easily produce
hundreds of KB. With multiple AI calls per routine and retries, logs
can explode to many MB.

**Per-attempt log files**: Each retry attempt gets its own log file.
The first attempt writes to `routine.log`, subsequent attempts write
to `routine-2.log`, `routine-3.log`, etc. This prevents retry
accumulation and makes it easy to compare attempts.

**Tail-truncation**: After each routine execution completes (success
or failure), decree checks the log file size against `max_log_size`
(default: 2MB). If the log exceeds the limit, truncate from the head
and prepend a marker line:

```
[log truncated — showing last 2MB of output]
```

The tail is kept because the end of execution (errors, final output)
is most useful for debugging. Real-time terminal output is unaffected —
truncation only applies to the saved log file.

**Config**: `max_log_size` in `config.yml` (bytes, default: `2097152`
= 2MB). Set to `0` to disable truncation.

### Ctrl-C / SIGINT

The routine's bash process must be spawned as a child in decree's
process group so that Ctrl-C propagates to it and all its children
(including the AI CLI subprocess). Specifically:

- Do **not** use `setsid` or create a new process group for the routine
- Do **not** spawn the routine in a way that detaches it from the
  terminal's process group
- SIGINT from Ctrl-C must reach the routine's bash process AND any
  subprocesses it spawned (e.g. `opencode run`, `claude -p`)
- When the routine is killed, decree should treat it as a failure
  (same as a non-zero exit) and proceed to the retry/dead-letter logic
- Decree itself should remain running after the routine is killed —
  it handles cleanup and moves to the next message

If the user hits Ctrl-C twice (or holds it), decree should exit
entirely after cleaning up the current run directory.

## Message Directory Structure

```
.decree/runs/
├── 2025022514320000-0/
│   ├── message.md
│   ├── routine.log           # attempt 1
│   ├── routine-2.log         # attempt 2 (if retried)
│   └── failure-context.md    # written before final retry
├── 2025022514320000-1/
│   └── ...
└── 2025022514321500-0/
    └── ...
```

## Acceptance Criteria

- [ ] `decree process` processes all unprocessed migrations in order
- [ ] After migrations, `decree process` drains inbox of remaining messages
- [ ] Bare `decree` is equivalent to `decree process`
- [ ] Each migration gets a new chain ID
- [ ] Run directory `.decree/runs/<chain>-<seq>/` is created per message
- [ ] Routine output streams to terminal in real-time via `tee`
- [ ] Routine output is simultaneously logged to `routine.log`
- [ ] Each retry attempt writes to a separate log (`routine.log`, `routine-2.log`, etc.)
- [ ] Logs exceeding `max_log_size` are tail-truncated with a marker line
- [ ] `max_log_size: 0` disables truncation
- [ ] Ctrl-C kills the routine and its AI subprocess immediately
- [ ] Decree remains running after Ctrl-C kills a routine (treats as failure)
- [ ] Double Ctrl-C exits decree entirely
- [ ] Lifecycle hooks run in correct order around each message
- [ ] Follow-up messages are processed depth-first within chains
- [ ] Non-final retry keeps partial changes in place
- [ ] Final retry reverts via git when stash hooks are active
- [ ] Exhausted messages are dead-lettered, processing continues
- [ ] `MaxDepthExceeded` dead-letters the message
- [ ] Dead-lettered messages can be re-queued by moving back to inbox
- [ ] Empty message bodies are accepted and processed normally
