# 03: Message Format and Normalization

## Overview

Defines the migration file format, the processed tracker, inbox message
formats, and the normalization pipeline that fills missing fields before
execution. Normalization is deterministic for most fields and uses the
AI only for routine selection when no routine is specified.

## Migration File Format

Migration files live in `.decree/migrations/`. Naming
convention: `NN-descriptive-name.md` (e.g., `01-add-auth.md`).

Each migration has optional YAML frontmatter:

```yaml
---
routine: develop
---
```

Migrations may also include **custom frontmatter fields** that are
passed through as env vars to the routine:

```yaml
---
routine: develop
project_name: myapp
target_branch: feature/auth
---
```

Custom fields are preserved through the entire pipeline: migration →
inbox message → routine execution (as env vars). Any frontmatter key
that is not a known field (`routine`) is treated as a custom field.

Followed by markdown content describing the work. The message body may
be empty — empty messages are valid (useful when the routine doesn't
need a prompt).

## Processed Tracker (`.decree/processed.md`)

A plain text file with one line per successfully processed migration:

```
01-add-auth.md
02-add-database.md
```

Processing logic:
1. Read `.decree/processed.md` (create if missing)
2. List all `*.md` files in `.decree/migrations/` alphabetically
3. Skip any whose filename appears in processed.md
4. Process the next unprocessed migration
5. On success: append filename to `processed.md`

## Inbox Message Format

Messages are markdown files in `.decree/inbox/`. YAML frontmatter is
**optional**. The processor normalizes every message before execution.

### Fully structured message (from migration):

```yaml
---
id: D0001-1432-01-add-auth-0
chain: D0001-1432-01-add-auth
seq: 0
routine: develop
migration: 01-add-auth.md
---
# 01: Add Auth

## Overview
Add authentication to the API.

## Requirements
- JWT-based auth middleware
- Login and register endpoints

## Acceptance Criteria
- [ ] Auth middleware rejects unauthenticated requests
- [ ] Login returns a valid JWT
```

### Follow-up message (from outbox):

Routines write files to `.decree/outbox/`. Decree assigns chain, seq,
and id when moving them to the inbox. Frontmatter is optional:

```
Fix type errors in src/auth.rs.
```

```yaml
---
routine: rust-develop
---
Fix type errors in src/auth.rs.
```

### Fields (all optional in raw message):

- `id` — full message ID: `<chain>-<seq>` (e.g., `D0001-1432-01-add-auth-0`)
- `chain` — chain ID: `D<NNNN>-HHmm-<name>` shared by all messages in the chain
- `seq` — sequence number (0 for root; incremented by decree for outbox follow-ups)
- `routine` — which routine to execute
- `migration` — original migration filename (set by decree, not user)
- Custom fields are passed as env vars to routines

Note: for ad-hoc runs (via `decree routine`), the `<name>` portion of the
chain ID is the routine name (e.g., `D0001-0900-develop-0`).

## Message Normalization

Before processing, the processor normalizes every inbox message.

### Field derivation (no AI needed):

1. **From filename**: `<chain>-<seq>.md` → extract `chain` and `seq`
2. **`chain`**: frontmatter → filename → generate new
3. **`seq`**: frontmatter → filename → default `0`
4. **`id`**: always `<chain>-<seq>` (recomputed)

Note: Outbox messages get `chain` and `seq` assigned by decree before
entering the inbox (see migration 04/07). Normalization only applies
to messages already in the inbox.

### Routine selection (AI-assisted):

If `routine` is missing:
1. List routines from `.decree/routines/` (including nested directories)
2. Extract descriptions from comment headers
3. Read `.decree/router.md` and populate `{routines}` (name +
   description per routine) and `{message}` (the message body)
4. Send populated prompt to `commands.ai_router`, parse response as routine name
5. Fallback chain: config default → "develop"

### Normalization output:

After normalization, the message file is rewritten with complete YAML
frontmatter and original body preserved. Bare messages get full frontmatter
added. Complete messages are not rewritten.

## Message File Naming

```
.decree/inbox/<chain>-<seq>.md
```

## Acceptance Criteria

- [ ] Migration files in `.decree/migrations/` are discovered alphabetically
- [ ] Processed migrations in `processed.md` are skipped
- [ ] Empty message bodies are accepted as valid
- [ ] Messages with no frontmatter get chain/seq derived from filename
- [ ] Messages with partial frontmatter get missing fields filled
- [ ] Router AI selects routine using `.decree/router.md` template
- [ ] Router fallback chain: frontmatter → config default → "develop"
- [ ] Normalization rewrites incomplete messages with full frontmatter
- [ ] Complete messages are not rewritten
- [ ] Custom frontmatter fields are preserved through normalization
- [ ] No `type` field exists — all messages are processed identically
- [ ] `migration` field is set by decree for messages created from migration files
- [ ] Custom frontmatter fields from migration files are propagated to inbox messages
- [ ] Custom fields survive the full pipeline: migration → inbox → routine env vars
