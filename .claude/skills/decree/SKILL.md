---
name: decree
description: Work within the Decree migration ecosystem safely and idiomatically
---

# Decree Skill for Claude

Decree is an AI orchestrator for structured, reproducible workflows. This skill
teaches you how to work within the Decree migration ecosystem safely and
idiomatically.

**Invoking this skill in Claude Code:** When installed at
`.claude/skills/decree/SKILL.md` (project scope) or
`~/.claude/skills/decree/SKILL.md` (user scope), this skill is available as the
`/decree` slash command. Invoking it loads this guidance into the session context.

**Invoking this skill in GitHub Copilot:** Skills are loaded automatically from
`.github/copilot-instructions.md`; there is no slash-command invocation.

## Migration Model

Decree uses ordered migration files stored in `.decree/migrations/`. Migrations
are:

- **Ordered by filename**: Migrations are processed in lexicographic filename
  order. Use a numeric prefix (e.g., `01-add-feature.md`, `02-refactor.md`) to
  control ordering.
- **Immutable once processed**: Once a migration is recorded in
  `.decree/processed.md`, do not edit it. Create a new migration instead.
- **Self-contained**: Each migration must be independently implementable without
  depending on the runtime state of sibling migrations.
- **Testable**: Each migration must include BDD acceptance criteria
  (Given / When / Then) that an automated routine can verify.

## Migration Format

Migration files are Markdown with optional YAML frontmatter:

```markdown
---
routine: develop
---

# Migration Title

Brief description of what this migration implements.

## Acceptance Criteria

- **Given** <precondition>
  **When** <action>
  **Then** <expected, verifiable result>
```

The `routine:` frontmatter field names the routine that should process this
migration. When omitted, the project's `default_routine` from `config.yml` is
used.

## Where to Place New Migrations

New migration files belong in `.decree/migrations/`. Choose the next available
numeric prefix to maintain ordering. Never place them in `.decree/processed.md`
or in any other location.

## Writing High-Quality Acceptance Criteria

Use the Given / When / Then format:

- **Given** — the precondition or initial state
- **When** — the action or trigger
- **Then** — the expected, verifiable outcome

Each criterion should be automatable. A routine running the codebase should be
able to verify it passes. Avoid vague outcomes such as "works correctly". Instead
specify observable behavior: exit codes, file contents, stdout output, HTTP
responses, or database state.

## Migration Sizing — Day-Sized, One Concern at a Time

Keep migrations **day-sized**: each migration should be implementable in a
single AI context window, completable in one session, and independently
reviewable.

**Split work into the smallest feasible logical chunks — one migration per
concern.** A migration that spans five subsystems is five migrations. Smaller
migrations:

- Reduce partial-failure risk — a failed migration can be retried without
  undoing unrelated work.
- Keep the AI's focus narrow enough to produce correct, reviewable output.
- Are easier to retry, debug, and verify individually.
- Produce cleaner commit history and review artifacts.

Think of migrations like developer tickets: scope them the way you would scope a
focused ticket that one engineer could complete and review in a day.

## Immutability Rule

Never edit a processed migration. The processed list lives in
`.decree/processed.md`. If a processed migration needs correction:

1. Create a new migration with the next available numeric prefix.
2. Describe what the new migration fixes and why the previous one was
   insufficient.
3. The new migration supersedes the old one; both remain in the repository.

## Ecosystem Integration

Keep all changes aligned with the migration workflow:

- New scripts, documentation, and Markdown files should be introduced through a
  migration with associated acceptance criteria, not applied directly to the
  repository outside of any migration.
- Each change — implementation, configuration, schema, or documentation — belongs
  in its own dedicated migration.
- If adding a script that later migrations will use, create a migration for the
  script first so it exists when those migrations run.
- Do not bypass the migration workflow by writing code directly to the repository
  without a corresponding migration and acceptance criteria.

## Processing Pipeline

Migrations are processed in the following order:

1. Migration files in `.decree/migrations/` are read in alphabetical order.
2. Each migration becomes an inbox message in `.decree/inbox/`.
3. Messages are normalized — missing fields are filled in and the routine is
   selected.
4. Lifecycle hooks run (`beforeEach` — e.g. git stash baseline).
5. The selected routine executes with parameters as environment variables.
6. On success: the `afterEach` hook runs, the message is deleted from the inbox,
   and `run.json` is written to the run directory with metadata about the
   completed run.
7. On failure: the retry strategy applies (hooks handle state management). If the
   run log contains "usage limit" + "reset", Decree waits until the reset time
   (SIGINT-aware, exits 130) then retries the migration from scratch.
8. After all retries: the message is dead-lettered. If the dead-lettered message
   was a migration, Decree stops immediately and exits non-zero — subsequent
   migrations are not started.
9. Follow-up messages from routines are processed depth-first.
10. The inbox is fully drained before the next migration starts.

## Environment Variables

Decree sets these variables before running every routine and hook:

| Variable      | Description                                              |
|---------------|----------------------------------------------------------|
| `message_file` | Path to `message.md` in the run directory              |
| `message_id`  | Full message ID (e.g., `D0001-1432-01-add-auth-0`)      |
| `message_dir` | Run directory path (contains logs from prior attempts)  |
| `chain`       | Chain ID (`D<NNNN>-HHmm-<name>`)                        |
| `seq`         | Sequence number in the chain                            |

Hook-only variables (set during hook execution):

| Variable                  | Description                                                   |
|---------------------------|---------------------------------------------------------------|
| `DECREE_HOOK`             | Hook type name (`beforeAll`, `afterAll`, etc.)                |
| `DECREE_ATTEMPT`          | Current attempt number (`beforeEach`/`afterEach`)             |
| `DECREE_MAX_RETRIES`      | Configured max retries (`beforeEach`/`afterEach`)             |
| `DECREE_ROUTINE_EXIT_CODE` | Routine exit code (`afterEach` only)                         |
| `DECREE_PRE_CHECK`        | Set to `"true"` during pre-check runs                         |
| `DECREE_FINAL_ATTEMPT`    | `"true"` on the final retry attempt (`afterEach` only)        |
| `DECREE_TRIGGER`          | How the run was initiated: `inbox`, `cron:<stem>`, or `chain` |

Retry variables (set on token-exhaustion retry):

| Variable                      | Description                                              |
|-------------------------------|----------------------------------------------------------|
| `DECREE_PREVIOUS_SESSION_ID`  | Claude session ID from the prior attempt, if captured   |

Custom frontmatter fields are also passed as environment variables.

## Defining Routines

Routines are shell scripts in `.decree/routines/` (nested directories allowed).
They invoke AI tools directly — Decree passes context via environment variables
and does not inject any magic.

Required script structure:

```bash
#!/usr/bin/env bash
# Title
#
# Description shown by `decree routine`.
# Additional lines shown in detail view.
set -euo pipefail

# --- Standard Environment Variables ---
# message_file  - Path to message.md in the run directory
# message_id    - Full message ID (e.g., D0001-1432-01-add-auth-0)
# message_dir   - Run directory path (contains logs from prior attempts)
# chain         - Chain ID (D<NNNN>-HHmm-<name>)
# seq           - Sequence number in chain
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"

# Pre-check (required — exit 0 if ready, non-zero if not):
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then
    command -v claude >/dev/null 2>&1 || { echo "claude not found" >&2; exit 1; }
    exit 0
fi

# Custom params (from frontmatter, discovered automatically):
my_param="${my_param:-default}"

# --- Implementation ---
claude -p "Read ${message_file} and implement the requirements.
Previous attempt logs (if any) are in ${message_dir} for context."
```

Routines call AI tools directly — write the exact command you want:

```bash
claude   -p "Read ${message_file} and implement the requirements."
copilot  -p "Read ${message_file} and implement the requirements."
opencode run "Read ${message_file} and implement the requirements."
```

### Custom Parameter Discovery

Decree scans the routine from top to bottom:
1. Skips: shebang, comments, blanks, `set` builtins, pre-check block
2. Matches: `var_name="${var_name:-default_value}"`
3. Stops at first non-matching line after the pre-check block
4. Excludes standard parameter names
5. Remaining variables are custom parameters — values come from message frontmatter
6. Empty defaults (`${var:-}`) mean optional with no default

### Pre-Check Section

Every routine must include a pre-check gate:
- Gate on `DECREE_PRE_CHECK=true` env var
- Place after standard params, before custom params
- Exit 0 = routine is ready, exit non-zero = not ready
- Print missing dependency to stderr on failure
- Used by `decree routine <name>` and `decree verify`

### Nested Routines

Routines can be organized in subdirectories:

```
.decree/routines/
├── develop.sh           # routine: develop
├── deploy/
│   ├── staging.sh       # routine: deploy/staging
│   └── production.sh    # routine: deploy/production
└── review/
    └── pr.sh            # routine: review/pr
```

### Tips

- **Pre-check required**: Every routine must have a pre-check section.
- **`set -euo pipefail`**: Always include — Decree treats non-zero exit as failure.
- **Run directory**: Use `${message_dir}` for logs and context from prior attempts.
- **stderr for pre-check errors**: Pre-check failures should print the missing dependency to stderr.
- **Routines are non-interactive**: They must run without user input.

## Routine Registry & Shared Routines

Routines are registered in `config.yml` under `routines` (project-local) and
`shared_routines` (shared library). A routine must be registered and enabled to
be discoverable or executable. Deprecated routines are treated as disabled.

Config layout:

```yaml
routine_source: "~/.decree/routines"    # shared routines directory
max_retries: 3                          # global default
routines:
  develop:
    enabled: true
  gmail-sync:
    enabled: true
    max_retries: 5        # overrides global max_retries for this routine
  actual-budget:
    enabled: true
    timeout_s: 60         # kill with SIGTERM after 60 s; treated as exit 1
shared_routines:
  deploy:
    enabled: true
```

Per-routine overrides:
- `max_retries` — overrides the global retry cap for that routine only.
- `timeout_s` — if set, the routine process is killed with SIGTERM after the
  given number of seconds and the attempt is treated as exit code 1.

Directory layering (first match wins):
1. `.decree/routines/` — project-local
2. `routine_source` path — shared library fallback

The same layering applies to prompts (`.decree/prompts/` vs shared).

Discovery runs automatically at `decree process`, `decree daemon`, and
`decree init`. New project routines are registered as enabled; new shared
routines as disabled. Routines whose files disappear are marked deprecated.
Hooks bypass the registry — they only need the script to exist on disk.

## Lifecycle Hooks

Hooks are configured in `config.yml`:

```yaml
hooks:
  beforeAll: ""      # Routine to run before all processing
  afterAll: ""       # Routine to run after all processing
  beforeEach: ""     # Routine to run before each message
  afterEach: ""      # Routine to run after each message
  onDeadLetter: ""   # Routine to run when a message is dead-lettered
```

Firing semantics:
- `beforeAll` / `afterAll` — fire once per `decree process` run.
- `beforeEach` / `afterEach` — fire before and after every message attempt.
- `onDeadLetter` — fires exactly once when a message moves to `inbox/dead/`
  after exhausting all retries. Does not fire on `beforeEach` failures.

Hooks receive the standard environment variables plus:

| Variable                    | Description                                                  |
|-----------------------------|--------------------------------------------------------------|
| `DECREE_HOOK`               | Hook type name                                               |
| `DECREE_ATTEMPT`            | Current attempt number (`beforeEach`/`afterEach`)            |
| `DECREE_MAX_RETRIES`        | Configured max retries (`beforeEach`/`afterEach`)            |
| `DECREE_ROUTINE_EXIT_CODE`  | Routine exit code (`afterEach` only)                         |
| `DECREE_FINAL_ATTEMPT`      | `"true"` in `afterEach` on the last attempt only             |
| `DECREE_TRIGGER`            | How the run was initiated (`inbox`, `cron:<stem>`, `chain`)  |

`onDeadLetter` additional variables:
- `DECREE_ATTEMPT` — equals `max_retries` (all retries were exhausted).
- `DECREE_MAX_RETRIES` — configured max retries.
- `DECREE_ROUTINE_EXIT_CODE` — exit code of the last attempt.
- `DECREE_TRIGGER` — how the run was initiated.

## Cron Scheduling

Cron-triggered messages are `.md` files placed in `.decree/cron/` with a `cron`
frontmatter field containing a standard cron expression:

```markdown
---
cron: "0 9 * * 1-5"
routine: develop
---
Run the weekday morning task.
```

Common schedule expressions:

| Expression      | Meaning                   |
|-----------------|---------------------------|
| `* * * * *`     | Every minute              |
| `0 * * * *`     | Every hour                |
| `0 9 * * *`     | Daily at 9:00 AM          |
| `0 9 * * 1-5`   | Weekdays at 9:00 AM       |
| `0 0 * * 0`     | Weekly on Sunday          |
| `0 0 1 * *`     | Monthly on the 1st        |
| `*/15 * * * *`  | Every 15 minutes          |

The `decree daemon` process monitors `.decree/cron/` and `.decree/inbox/`
continuously. The `decree cron list` command inspects live schedule status
(last run, next fire time).

## run.json

After every completed run (success or dead-letter), Decree writes `run.json` to
the run directory. Fields:

| Field         | Description                                                      |
|---------------|------------------------------------------------------------------|
| `message_id`  | Full message ID                                                  |
| `routine`     | Routine name used for processing                                 |
| `trigger`     | How the run was initiated (`inbox`, `cron:<stem>`, `chain`)      |
| `migration`   | Migration filename, if this was a migration run                  |
| `attempts`    | Number of attempts made                                          |
| `exit_code`   | Exit code of the final attempt                                   |
| `start`       | ISO-8601 timestamp when the run started                          |
| `end`         | ISO-8601 timestamp when the run ended                            |
| `duration_s`  | Total elapsed seconds                                            |
