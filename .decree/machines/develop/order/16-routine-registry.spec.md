---
routine: rust-develop
---

# 16: Routine Registry and Shared Routines

## Overview

Make `.decree/config.yml` the single source of truth for which routines
are available. Routines must be registered in config and enabled to be
usable. Add support for a shared routines directory that acts as a
library — project-local `.decree/routines/` takes precedence, with the
shared directory as a fallback. The same layering applies to prompts.

## Requirements

### Routine registry in config.yml

Add a `routines` section to config.yml that explicitly lists project-local
routines with their enabled/disabled status. Add a separate
`shared_routines` section for routines from the shared library.

A routine must be registered AND enabled to be discoverable or executable.
A routine marked `deprecated` is treated as disabled regardless of its
`enabled` field.

When the `routines` section is absent from config (legacy projects),
all project-local filesystem routines are available for backward
compatibility. Once the section exists, it becomes the strict gate.

`shared_routines` is always strict — shared routines must be explicitly
enabled.

Config layout (routines after hooks, shared_routines at the end):

```yaml
commands:
  ai_router: "claude -p {prompt}"
  ai_interactive: "claude"

max_retries: 3
max_depth: 10
max_log_size: 2097152
default_routine: develop
routine_source: "~/.decree/routines"

hooks:
  beforeAll: ""
  afterAll: ""
  beforeEach: "git-baseline"
  afterEach: "git-stash-changes"

routines:
  develop:
    enabled: true
  rust-develop:
    enabled: true
  git-baseline:
    enabled: true
  git-stash-changes:
    enabled: true

shared_routines:
  deploy:
    enabled: true
  notify:
    enabled: false
  data-pipeline:
    enabled: false
```

Each routine entry has two fields:

- `enabled` — whether the routine can be used. Defaults to `true`.
- `deprecated` — set by discovery when a routine's file no longer exists.
  Treated as disabled regardless of `enabled`. Defaults to `false`.

### Shared routines directory

Add a `routine_source` config field pointing to a shared routines
directory. The conventional path is `~/.decree/routines`, mirroring the
project-level `.decree/routines/` at the user level.

Decree must expand a leading `~` in `routine_source` to the current
user's home directory at runtime. This makes the config value portable:

```yaml
routine_source: "~/.decree/routines"
```

On a developer machine, `~` expands to `/home/<user>`. In a Docker
container running as root, it expands to `/root`. Same config, same
logical path.

When `routine_source` is not set, only project-local routines are used.

### Directory layering

When resolving a routine, decree checks directories in this order:

1. `.decree/routines/` (project-local — wins if present)
2. The shared directory from `routine_source` (fallback)

To customize a shared routine for a project, copy it into
`.decree/routines/` and modify it there. The local version takes
precedence automatically.

A routine must be enabled in the appropriate config section to be
resolved — `routines` for project-local, `shared_routines` for shared.
If a routine exists on disk but is not enabled (or not registered), it
cannot be found or executed.

Hooks bypass the registry. Hook routines referenced in the `hooks`
config section must exist on disk but do not need to be registered or
enabled. This avoids a circular dependency.

### Prompt layering

Apply the same directory layering to prompts. When `routine_source` is
set, derive the shared prompts path as the sibling directory: if
`routine_source` is `~/.decree/routines`, the shared prompts path is
`~/.decree/prompts`.

Prompt resolution checks `.decree/prompts/` first, then the shared
prompts directory. Prompts do NOT require config registration — any
prompt file in either directory is available.

### Discovery

Discovery scans both directories and registers any new routines into
config. It runs at these checkpoints:

1. **`decree init`** — register routines created during init.
2. **`decree process`** — discover once at start, before hooks and
   processing.
3. **`decree daemon`** — discover once at startup.

Discovery behavior:

- Scan `.decree/routines/` for `.sh` files. For each new routine not
  already in `routines`: add with `enabled: true`.
- If `routine_source` is set, scan the shared directory. For each new
  routine not already in `shared_routines`: add with `enabled: false`.
- For each registered routine whose file no longer exists in its
  respective directory: set `deprecated: true`.
- For each deprecated routine whose file has reappeared: clear
  `deprecated`.
- Discovery never removes entries and never changes the `enabled` field
  on existing entries.
- If no changes are detected, config is not rewritten.

### `decree routine-sync` command

Add a `routine-sync` CLI command that runs discovery manually and
displays the results. Useful after adding routines to the shared
library, without waiting for the next process/daemon cycle.

```
decree routine-sync [--source <dir>]
```

`--source` overrides `config.routine_source`. If neither is set, only
project-local discovery runs.

Output lists all routines with their status:

```
Project routines (.decree/routines/):
  develop         enabled
  rust-develop    enabled

Shared routines (~/.decree/routines/):
  deploy          enabled
  notify          disabled (new)
  data-pipeline   disabled
```

### `decree init` changes

When generating config.yml, include:

- A `routines` section listing all routines created during init as
  `enabled: true`.
- A commented-out `routine_source` hint: `# routine_source: "~/.decree/routines"`
- If `~/.decree/routines/` exists and contains `.sh` files, populate a
  `shared_routines` section with those routines as `enabled: false`.

### Routine listing and selection

All places that list available routines (interactive routine selection,
AI router prompt, pre-check verification) must respect the registry:

- When the `routines` section exists, only enabled project-local routines
  are listed.
- When `routine_source` is set, enabled shared routines are also included.
- When the `routines` section is absent (legacy), all project-local
  filesystem routines are listed.

Routine resolution at execution time must also check the registry.
If a message targets a disabled routine, it should be dead-lettered
with a clear "routine disabled" error.

### Help text

Add `routine-sync` to the command list and add documentation sections
for routine sync, shared routines, and the directory layering model.

### Docker integration

The Docker entrypoint no longer needs to call a sync script. The daemon
handles discovery automatically via `routine_source` in config.

Remove the `routine-sync.sh` script, its test harness, and the `yq`
dependency from the Docker image. The decree binary handles all YAML
operations natively.

The Docker compose example mounts the host's shared routines:

```yaml
volumes:
  - .:/work
  - ~/.decree/routines:~/.decree/routines
```

Update the Dockerfile to remove the `yq` installation step and the
COPY/chmod of `routine-sync.sh`.

Update the `.dockerignore` to remove the `docker/test-routine-sync.sh`
entry.

## Files to Create

- Routine sync command implementation (discovery logic + CLI output)

## Files to Delete

- `docker/routine-sync.sh` — replaced by native discovery + layered resolution
- `docker/test-routine-sync.sh` — replaced by Rust integration tests

## Acceptance Criteria

### Config

- **Given** a config.yml with `routines` and `shared_routines` sections
  **When** the config is loaded
  **Then** both sections are parsed with correct enabled/disabled/deprecated
  states

- **Given** a config.yml without a `routines` section (legacy format)
  **When** the config is loaded
  **Then** all project-local filesystem routines are available

- **Given** `routine_source: "~/.decree/routines"` in config
  **When** the path is resolved
  **Then** the leading `~` is expanded to the current user's home directory

### Routine enablement

- **Given** `develop` is enabled in `routines` and `rust-develop` is
  disabled
  **When** routines are listed
  **Then** only `develop` appears

- **Given** `deploy` is enabled in `shared_routines`
  **When** routines are listed
  **Then** `deploy` appears alongside enabled project-local routines

- **Given** `notify` is disabled in `shared_routines`
  **When** routines are listed
  **Then** `notify` does not appear

- **Given** a routine marked `deprecated: true`
  **When** routines are listed
  **Then** it does not appear regardless of `enabled`

### Directory layering

- **Given** `develop.sh` exists in both `.decree/routines/` and
  `~/.decree/routines/`
  **When** the routine is resolved
  **Then** the project-local version is used

- **Given** `deploy.sh` exists only in `~/.decree/routines/` and is
  enabled in `shared_routines`
  **When** the routine is resolved
  **Then** the shared version is used

- **Given** `deploy.sh` exists in `~/.decree/routines/` but is NOT
  enabled in `shared_routines`
  **When** the routine is resolved
  **Then** it fails with a "routine disabled" error

- **Given** `routine_source` is not set
  **When** routines are resolved
  **Then** only `.decree/routines/` is searched

### Discovery

- **Given** a new `audit.sh` is added to `.decree/routines/`
  **When** `decree process` runs
  **Then** `audit` is added to `routines` with `enabled: true`

- **Given** a new `deploy.sh` is added to `~/.decree/routines/`
  **When** `decree process` runs
  **Then** `deploy` is added to `shared_routines` with `enabled: false`

- **Given** `deploy.sh` is removed from the shared directory
  **When** discovery runs
  **Then** `deploy` is marked `deprecated: true`

- **Given** a deprecated routine whose file reappears
  **When** discovery runs
  **Then** `deprecated` is cleared

- **Given** an existing entry with `enabled: true`
  **When** discovery runs
  **Then** `enabled` is not changed

- **Given** discovery finds no changes
  **When** it completes
  **Then** config.yml is not rewritten

### Init

- **Given** `decree init` is run
  **When** config.yml is generated
  **Then** the `routines` section lists all init-created routines as
  `enabled: true` and a commented-out `routine_source` hint is present

- **Given** `~/.decree/routines/` exists with `.sh` files during init
  **When** `decree init` is run
  **Then** `shared_routines` is populated with those routines as
  `enabled: false`

### Routine execution

- **Given** a disabled routine in config
  **When** `decree process` runs a message targeting that routine
  **Then** the message is dead-lettered with a "routine disabled" error

- **Given** a hook routine not in the routines registry
  **When** the hook is triggered during processing
  **Then** the hook executes normally (hooks bypass the registry)

### AI router

- **Given** config with routines and shared_routines sections
  **When** the AI router builds the routine selection prompt
  **Then** only enabled routines from both sections appear

### `decree routine-sync`

- **Given** project and shared directories with routines
  **When** `decree routine-sync` is run
  **Then** new routines are registered (project as enabled, shared as
  disabled) and all routines are listed with their status

- **Given** `--source ~/other-routines` is passed
  **When** `decree routine-sync` runs
  **Then** the specified directory is used instead of `routine_source`

- **Given** neither `--source` nor `routine_source` is set
  **When** `decree routine-sync` is run
  **Then** only project-local discovery runs

### Prompt layering

- **Given** a prompt exists in both `.decree/prompts/` and
  `~/.decree/prompts/`
  **When** the prompt is resolved
  **Then** the project-local version is used

- **Given** a prompt exists only in `~/.decree/prompts/`
  **When** the prompt is resolved
  **Then** the shared version is used

### Docker

- **Given** Docker compose with `~/.decree/routines:~/.decree/routines`
  mounted and `routine_source` set in config
  **When** the daemon starts
  **Then** discovery and layered resolution work identically to bare metal

- **Given** the Docker image
  **When** it is built
  **Then** `yq` is not installed and `routine-sync.sh` is not present

### Backward compatibility

- **Given** an existing project with no `routines` key and no
  `routine_source` in config
  **When** any decree command is run
  **Then** all project-local filesystem routines are available (identical
  to pre-migration behavior)
