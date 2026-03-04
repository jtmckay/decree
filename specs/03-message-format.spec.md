# 03: Message Format and Normalization

## Overview

Defines the migration file format, the processed tracker, inbox message
formats, and the normalization pipeline that fills missing fields before
execution. Normalization is deterministic for most fields and uses the
AI only for routine selection when no routine is specified.

## Migration File Format

Migration files live in `migrations/` at the project root. Naming
convention: `NN-descriptive-name.md` (e.g., `01-add-auth.md`).

Each migration has optional YAML frontmatter:

```yaml
---
routine: develop
---
```

Followed by markdown content describing the work. The message body may
be empty — empty messages are valid (useful when `input_file` is the
source of truth or when the routine doesn't need a prompt).

## Processed Tracker (`migrations/processed.md`)

A plain text file with one line per successfully processed migration:

```
01-add-auth.md
02-add-database.md
```

Processing logic:
1. Read `migrations/processed.md` (create if missing)
2. List all `*.md` files in `migrations/` alphabetically (excluding `processed.md`)
3. Skip any whose filename appears in processed.md
4. Process the next unprocessed migration
5. On success: append filename to `processed.md`

## Inbox Message Format

Messages are markdown files in `.decree/inbox/`. YAML frontmatter is
**optional**. The processor normalizes every message before execution.

### Fully structured message:

```yaml
---
id: 2025022514320000-0
chain: 2025022514320000
seq: 0
type: spec
input_file: migrations/01-add-auth.md
routine: develop
---
```

### Minimal message (routine-spawned follow-up):

```yaml
---
routine: develop
---
Fix type errors in src/auth.rs.
```

### Bare message (no frontmatter):

```
Fix type errors in src/auth.rs.
```

### Empty message (no body, frontmatter only):

```yaml
---
input_file: migrations/01-add-auth.md
routine: develop
---
```

### Fields (all optional in raw message):

- `id` — full message ID: `<chain>-<seq>`
- `chain` — base ID shared by all messages in the chain
- `seq` — sequence number (0 for root, incrementing)
- `type` — `spec` or `task`
- `input_file` — path to input file
- `routine` — which routine to execute
- Custom fields are passed as env vars to routines

## Message Normalization

Before processing, the processor normalizes every inbox message.

### Field derivation (no AI needed):

1. **From filename**: `<chain>-<seq>.md` → extract `chain` and `seq`
2. **`chain`**: frontmatter → filename → generate new
3. **`seq`**: frontmatter → filename → default `0`
4. **`id`**: always `<chain>-<seq>` (recomputed)
5. **`type`**: frontmatter → `spec` if `input_file` set → `task`
6. **`input_file`**: frontmatter if present

### Routine selection (AI-assisted):

If `routine` is missing:
1. List routines from `.decree/routines/` (including nested directories)
2. Extract descriptions from comment headers
3. Build router prompt with available routines and message body
4. Send to `commands.ai`, parse response as routine name
5. Fallback chain: migration frontmatter → config default → "develop"

### Normalization output:

After normalization, the message file is rewritten with complete YAML
frontmatter and original body preserved. Bare messages get full frontmatter
added. Complete messages are not rewritten.

## Message File Naming

```
.decree/inbox/<chain>-<seq>.md
```

## Acceptance Criteria

- [ ] Migration files in `migrations/` are discovered alphabetically
- [ ] Processed migrations in `processed.md` are skipped
- [ ] Empty message bodies are accepted as valid
- [ ] Messages with no frontmatter get chain/seq derived from filename
- [ ] Messages with partial frontmatter get missing fields filled
- [ ] Router AI selects routine when `routine` field is missing
- [ ] Router fallback chain: frontmatter → config default → "develop"
- [ ] Normalization rewrites incomplete messages with full frontmatter
- [ ] Complete messages are not rewritten
- [ ] Custom frontmatter fields are preserved through normalization
