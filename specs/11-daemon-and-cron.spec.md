# 11: Daemon and Cron

## Overview

`decree daemon` continuously monitors the inbox and cron directory,
processing messages as they appear. Cron files define scheduled tasks
that fire on a cron expression. Since `decree process` now drains the
inbox after processing migrations, the daemon is primarily useful for
long-running monitoring and scheduled tasks.

## `decree daemon`

```
decree daemon                    # poll every 2 seconds (default)
decree daemon --interval 5       # poll every 5 seconds
```

## Daemon Polling Loop

1. Check `.decree/cron/` for due cron jobs
2. Copy due jobs into `.decree/inbox/` as new messages
3. Check `.decree/inbox/` for pending messages
4. If messages found: process them (depth-first within chains)
5. Sleep for the interval
6. Go to step 1
7. Exit on SIGINT/SIGTERM (graceful shutdown after current message)

The daemon runs lifecycle hooks: `beforeAll` on startup, `afterAll`
on shutdown, `beforeEach`/`afterEach` around each message.

## Cron Directory

Cron files are `.md` files in `.decree/cron/` with a `cron` property
in YAML frontmatter. The `cron` value is a standard 5-field expression
(minute, hour, day-of-month, month, day-of-week).

### Cron File Format

```yaml
---
cron: "0 * * * *"
routine: develop
---
Run the hourly maintenance task.
```

Fields:
- `cron` (required) — standard 5-field cron expression
- `routine` (optional) — follows normal selection chain if omitted
- Any other frontmatter fields are passed through
- Body is the task description

## Cron Evaluation

On each poll, before checking the inbox:

1. Scan `.decree/cron/` for `*.md` files with `cron` frontmatter
2. Evaluate whether each expression matches the current minute
3. Track last fire time per file to avoid duplicates within same minute
4. For due jobs, create inbox message:
   - Generate new chain ID
   - Name: `<chain>-0.md`
   - Set `seq: 0`, `type: task`
   - Copy `routine` and custom fields (strip `cron` field)
   - Copy body as message body
5. Message is processed through standard pipeline

Cron files are **not consumed** — they stay and fire repeatedly.
Restarting the daemon resets tracking (may fire once on restart).

## Cron-to-Inbox Message

When a cron file fires:
1. Generate new chain ID
2. Create `.decree/inbox/<chain>-0.md`
3. Write frontmatter: `seq: 0`, `type: task`, `routine` (if present),
   custom fields — **strip the `cron` field**
4. Copy markdown body
5. Message is normalized, checkpointed, executed, and dispositioned
   like any other message

## Acceptance Criteria

- [ ] `decree daemon` polls inbox and cron at configurable interval
- [ ] Daemon processes inbox messages through standard pipeline
- [ ] Cron files with `cron` frontmatter are evaluated on each poll
- [ ] Due cron jobs create inbox messages with correct frontmatter
- [ ] `cron` field is stripped from the inbox message
- [ ] Custom frontmatter fields are preserved in cron-to-inbox copy
- [ ] Duplicate firings within same minute are prevented
- [ ] Cron files are never consumed (fire repeatedly)
- [ ] SIGINT/SIGTERM triggers graceful shutdown
- [ ] Follow-up messages are processed depth-first within chains
- [ ] Dead-lettered messages don't halt the daemon
- [ ] Daemon runs lifecycle hooks (beforeAll/afterAll/beforeEach/afterEach)
