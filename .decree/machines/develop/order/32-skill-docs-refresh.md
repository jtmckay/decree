---
routine: develop
---
# 32: Skill Documentation Refresh

## Overview

Update the bundled decree skill (`src/templates/skills/claude/SKILL.md`) to
include the full operational reference that an AI agent needs when working
inside a Decree project. The current skill covers the migration model well but
omits key runtime information: the processing pipeline, environment variables,
lifecycle hooks, cron scheduling, routine authoring, run.json metadata, and the
routine registry. Add a brief note about how to invoke the skill in supported
AI tools. Strip any text that describes installation mechanics — the skill
should read as a concise reference manual for an agent already running inside
the ecosystem.

Also update the migration format example in the skill (and anywhere else in
the templates) to use `develop` as the default `routine:` value instead of
`rust-develop`.

## Requirements

1. **Content to add** — incorporate the following sections from `help.txt` into
   the skill, adapted for an AI audience (declarative prose, not imperative
   "run this command" framing):
   - Processing pipeline (steps 1-10)
   - Environment variables set by Decree (standard + hook-only + retry)
   - Defining routines — required script structure, pre-check pattern, custom
     parameter discovery, tips
   - Routine registry & shared routines — config layout, per-routine overrides,
     directory layering, discovery rules
   - Lifecycle hooks — config keys, firing semantics, hook-specific env vars,
     onDeadLetter extras
   - Cron scheduling — frontmatter format, common schedule examples
   - run.json — fields and their meaning

2. **Skill invocation blurb** — add a short note near the top of the skill
   (after the introductory paragraph, before the Migration Model section)
   explaining how to invoke the skill in supported AI tools:
   - **Claude Code**: when installed at `.claude/skills/decree/SKILL.md`
     (project) or `~/.claude/skills/decree/SKILL.md` (user), the skill is
     available as the `/decree` slash command. Invoking it loads this guidance
     into the session context.
   - **GitHub Copilot**: skills are loaded automatically from
     `.github/copilot-instructions.md`; there is no slash-command invocation.
   Keep this to one short paragraph per assistant.

3. **Content to remove** — delete the "Repository Portability" section at the
   bottom of the skill. It describes installation mechanics irrelevant to an
   agent doing work.

4. **Default routine example** — change every occurrence of `rust-develop` in
   the skill template to `develop`. Apply the same change to any other template
   files that use `rust-develop` as an example value (e.g., `migration.md`).

5. **Tone** — write in declarative prose ("Migrations are processed in…") not
   tutorial prose ("Run `decree process` to…"). Remove user-facing CLI tips
   that belong in `help.txt`, not an agent skill.

## Files to Modify

- `src/templates/skills/claude/SKILL.md` — primary target; expand with runtime
  reference, add invocation blurb, remove portability section, fix default
  routine example
- `src/templates/migration.md` — change `rust-develop` example to `develop` if
  present

## Acceptance Criteria

- **Given** the file `src/templates/skills/claude/SKILL.md`
  **When** its content is inspected
  **Then** it contains a section describing how to invoke the skill in Claude
  Code via the `/decree` slash command

- **Given** the file `src/templates/skills/claude/SKILL.md`
  **When** its content is inspected
  **Then** it contains a "Processing Pipeline" section that lists the 10
  pipeline steps (migration → inbox → normalize → hooks → routine →
  success/retry/dead-letter)

- **Given** the file `src/templates/skills/claude/SKILL.md`
  **When** its content is inspected
  **Then** it contains an "Environment Variables" section listing at minimum
  `message_file`, `message_id`, `message_dir`, `chain`, `seq`,
  `DECREE_HOOK`, `DECREE_ATTEMPT`, `DECREE_MAX_RETRIES`,
  `DECREE_ROUTINE_EXIT_CODE`, `DECREE_FINAL_ATTEMPT`, `DECREE_TRIGGER`, and
  `DECREE_PREVIOUS_SESSION_ID`

- **Given** the file `src/templates/skills/claude/SKILL.md`
  **When** its content is inspected
  **Then** it contains a "Lifecycle Hooks" section describing the five hook keys
  (`beforeAll`, `afterAll`, `beforeEach`, `afterEach`, `onDeadLetter`) and
  their firing semantics

- **Given** the file `src/templates/skills/claude/SKILL.md`
  **When** its content is inspected
  **Then** it contains a "Cron Scheduling" section describing the `cron`
  frontmatter field and the `.decree/cron/` directory

- **Given** the file `src/templates/skills/claude/SKILL.md`
  **When** its content is inspected
  **Then** it contains a `run.json` section listing the fields
  (`message_id`, `routine`, `trigger`, `migration`, `attempts`, `exit_code`,
  `start`, `end`, `duration_s`)

- **Given** the file `src/templates/skills/claude/SKILL.md`
  **When** its content is inspected
  **Then** it does NOT contain a "Repository Portability" section

- **Given** the file `src/templates/skills/claude/SKILL.md`
  **When** its content is inspected
  **Then** the string `rust-develop` does not appear anywhere in the file

- **Given** the file `src/templates/migration.md` contains a `routine:` example
  **When** its content is inspected
  **Then** the example value is `develop`, not `rust-develop`
