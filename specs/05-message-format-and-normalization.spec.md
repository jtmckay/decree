---
routine: rust-develop
---

# 05: Message Format and Normalization

## Overview

Defines the spec file format, the processed-spec tracker, inbox message
formats (fully structured, minimal, and bare), and the normalization
pipeline that fills missing fields before execution. Normalization is
deterministic for most fields (derived from filenames, frontmatter, and
type inference) and uses the router AI only for routine selection when no
routine is specified.

## Requirements

### Spec File Format

Spec files live in `specs/` at the project root (adjacent to `.decree/`).
Naming convention: `NN-descriptive-name.spec.md` (e.g., `01-add-auth.spec.md`).

Each spec has optional YAML frontmatter:
```yaml
---
routine: develop          # which routine to use (optional, defaults to config)
---
```

Followed by markdown content describing the work.

### Processed Tracker (`specs/processed-spec.md`)

A plain text file with one line per successfully processed spec:
```
01-add-auth.spec.md
02-add-database.spec.md
```

Processing logic:
1. Read `specs/processed-spec.md` (create if missing)
2. List all `*.spec.md` files in `specs/` alphabetically
3. Skip any spec whose filename appears in processed-spec.md
4. Process the next unprocessed spec
5. On success: append filename to `processed-spec.md`
6. This spec will never be processed again (immutability)

### Inbox Message Format

Messages are markdown files in `.decree/inbox/`. YAML frontmatter is
**optional** — a message can be anything from a fully structured message
with all fields to plain text with no frontmatter at all. The processor
normalizes every message before execution (see "Message Normalization").

#### Fully structured message (all fields present):

```yaml
---
id: 2025022514320000-0
chain: 2025022514320000
seq: 0
type: spec
input_file: specs/01-add-auth.spec.md
routine: develop
---
```

#### Minimal message (routine-spawned follow-up, partial frontmatter):

```yaml
---
chain: 2025022514320000
seq: 1
---
Fix type errors in src/auth.rs introduced by the auth implementation.
```

#### Bare message (no frontmatter at all):

```
Fix type errors in src/auth.rs introduced by the auth implementation.
```

#### Fields (all optional in the raw message):

- `id` — full message ID: `<chain>-<seq>` (format defined in spec 01)
- `chain` — base ID shared by all messages in this chain
- `seq` — sequence number within the chain (0 for root, incrementing by 1)
- `type` — `spec` (from `decree run`/`decree process`) or `task` (from
  a routine)
- `input_file` — path to the input file (spec file path for `type: spec`;
  tasks may also reference a file). Custom fields beyond this standard set
  are valid in frontmatter — see spec 06 for custom routine variables.
- `routine` — which routine to execute

The message body (after frontmatter) is free-form markdown — for `spec`
messages it's ignored (the spec file is the source of truth), for `task`
messages it's the task description passed to the routine.

### Message Normalization

Before processing, the processor normalizes every inbox message to ensure
all required fields are present. This runs once when a message is picked
up — the normalized message is written back to the file before any
execution begins.

#### Field derivation (no AI needed):

1. **From filename**: if the file is named `<chain>-<seq>.md`, extract
   `chain` and `seq` from the filename. This is the primary source for
   messages spawned by routines that set the filename but skip frontmatter.
2. **`chain`**: use frontmatter if present, else derive from filename,
   else generate a new chain ID.
3. **`seq`**: use frontmatter if present, else derive from filename,
   else default to `0`.
4. **`id`**: always `<chain>-<seq>` — recomputed from the resolved chain
   and seq.
5. **`type`**: use frontmatter if present. If missing: `spec` when
   `input_file` is set, otherwise `task`.
6. **`input_file`**: use frontmatter if present. Only relevant for
   `type: spec`.

#### Routine selection (AI-assisted):

If the `routine` field is missing, the processor uses the **router AI**
(`commands.router`) to select the best routine:

1. List all available routines by scanning `.decree/routines/` for `*.sh`
   files, and also `*.ipynb` files when `notebook_support` is enabled.
   Extract the routine name (filename without extension). Deduplicate by
   name — if both `develop.sh` and `develop.ipynb` exist, list `develop`
   once.
2. Extract the description from each routine:
   - **Shell scripts**: the first block of contiguous `#` comment lines at
     the top of the script (after the optional shebang). Strip the leading
     `# ` (or lone `#` for blank comment lines) to produce description text.
   - **Notebooks**: the first markdown cell content.
   When both formats exist for a name, use the `.sh` description (it takes
   precedence).
3. Build a router prompt:

```
Select the most appropriate routine for this task.

## Available Routines
{for each routine: "- <name>: <first-line-of-description>"}

## Task
{message body text}

Respond with ONLY the routine name, nothing else.
```

4. Send the prompt to `commands.router` and parse the response as a
   routine name.
5. If the router returns an unrecognized name or fails, fall back to the
   standard routine selection chain: spec frontmatter → config default →
   "develop".

When the router is the embedded AI (`decree ai`), this runs in-process.
When the router is an external command, it runs as a one-shot subprocess.

#### Normalization output:

After normalization, the message file is rewritten with complete YAML
frontmatter and the original body preserved. A bare message like:

```
Fix type errors in src/auth.rs
```

Becomes:

```yaml
---
id: 2025022514320000-1
chain: 2025022514320000
seq: 1
type: task
routine: develop
---
Fix type errors in src/auth.rs
```

If the message already has complete frontmatter, normalization is a no-op
(the file is not rewritten).

### Message File Naming

```
.decree/inbox/<chain>-<seq>.md
```

Example: `2025022514320000-0.md`, then `2025022514320000-1.md`

## Acceptance Criteria

### Spec Processing

- **Given** multiple `*.spec.md` files exist in `specs/`
  **When** `decree process` runs
  **Then** specs are processed in alphabetical order, one at a time

- **Given** `specs/processed-spec.md` lists a spec as processed
  **When** `decree process` collects unprocessed specs
  **Then** that spec is skipped

- **Given** a new spec file is added after a previous run
  **When** `decree process` is invoked again
  **Then** the new spec is picked up and processed

- **Given** a spec has `routine: custom` in its YAML frontmatter
  **When** the inbox message for that spec is processed
  **Then** the routine is resolved via discovery (`.sh` first, then `.ipynb`)

### Message Normalization

- **Given** a message file has complete YAML frontmatter with all fields
  **When** the processor picks it up
  **Then** normalization is a no-op and the file is not rewritten

- **Given** a message file has no YAML frontmatter (bare text)
  **When** the processor picks it up
  **Then** `chain` and `seq` are derived from the filename
  **And** `type` is set to `task`
  **And** the router AI is invoked to select the routine
  **And** the file is rewritten with complete frontmatter and the original body

- **Given** a message file has partial frontmatter (e.g., only `chain` and `seq`)
  **When** the processor picks it up
  **Then** missing fields are filled in (type inferred, routine selected)
  **And** present fields are preserved as-is

- **Given** a message file has no `routine` field and multiple routines exist
  **When** normalization runs
  **Then** the router AI receives the message body and a list of available routines with descriptions
  **And** the router's response is used as the routine name

- **Given** the router AI returns an unrecognized routine name or fails
  **When** normalization falls back
  **Then** routine selection continues with spec frontmatter → config default → "develop"

- **Given** a message filename does not follow the `<chain>-<seq>.md` pattern
  **When** normalization cannot derive chain/seq from the filename
  **Then** a new chain ID is generated and seq defaults to 0
