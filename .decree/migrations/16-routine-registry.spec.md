---
routine: rust-develop
---

# 16: Routine Registry in config.yml

## Overview

Make `.decree/config.yml` the single source of truth for which routines are
available. Routines must be registered in config and enabled to be usable.

Routine resolution uses directory layering: project-local
`.decree/routines/` takes precedence over shared `~/.decree/routines/`.
No files are copied between directories — shared routines are used
in-place. To customize a shared routine for a project, copy it into
`.decree/routines/` manually; the local version wins.

Discovery of new routines happens at natural checkpoints (init, process,
daemon start). New project-local routines default to `enabled: true`. New
shared routines default to `enabled: false` — the user must explicitly
opt in.

The same layering applies to prompts: `.decree/prompts/` first, then
`~/.decree/prompts/`.

## Requirements

### Config structure (src/config.rs)

Add a `RoutineConfig` struct:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutineConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub deprecated: bool,
}
```

Fields:

- `enabled` — whether the routine can be discovered and executed. Default
  `true`.
- `deprecated` — set during discovery when a previously-registered routine
  no longer exists in either directory. A deprecated routine is treated as
  disabled regardless of `enabled`.

Add `routine_source` to `AppConfig`:

```rust
/// Path to shared routines directory (e.g., "~/.decree/routines").
/// Tilde is expanded at runtime. When set, decree resolves routines
/// from this directory as a fallback after .decree/routines/.
#[serde(default, skip_serializing_if = "Option::is_none")]
pub routine_source: Option<String>,
```

Add the routines registry as two separate sections:

```rust
/// Project-local routine registry. Routines in .decree/routines/.
/// None = absent from config (backward compat). Some = strict mode.
#[serde(default)]
pub routines: Option<BTreeMap<String, RoutineConfig>>,

/// Shared routine registry. Routines from routine_source directory.
/// Only used when routine_source is set.
#[serde(default, skip_serializing_if = "Option::is_none")]
pub shared_routines: Option<BTreeMap<String, RoutineConfig>>,
```

Using `Option<BTreeMap>` for `routines`:

- `None` (key absent from YAML) = backward-compatible mode, all project-
  local filesystem routines are available.
- `Some(map)` = strict mode, only listed+enabled routines are usable.

`shared_routines` is always strict — shared routines must be explicitly
enabled. It is only relevant when `routine_source` is set.

Add a helper method `is_routine_enabled(&self, name: &str) -> bool`:

- Returns `true` if `routines` is `None` (backward compat for project-
  local routines).
- Returns `true` if the routine is in `routines` map, `enabled` is true,
  and `deprecated` is false.
- Returns `true` if the routine is in `shared_routines` map, `enabled` is
  true, and `deprecated` is false.
- Returns `false` otherwise.

Add `resolve_routine_source(&self) -> Option<PathBuf>`:

- Returns `None` if `routine_source` is not set.
- Expands leading `~` or `~/` to the current user's home directory.
- Returns the expanded path.

Update `Default` for `AppConfig` to include `routines: None`,
`shared_routines: None`, `routine_source: None`.

#### Tilde expansion

When loading `routine_source`, decree must expand a leading `~` or `~/` to
the current user's home directory (via `std::env::var("HOME")` or
`dirs::home_dir()`). This allows the same config value to resolve
correctly across environments.

#### Standard shared routines path

The conventional path for shared routines is `~/.decree/routines`. This
mirrors the project-level `.decree/routines` at the user level.

- **Bare metal**: resolves to the current user's home.
- **Docker**: resolves to root's home. Mount
  `~/.decree/routines:~/.decree/routines` in compose.

#### Config YAML layout

The `routines` section appears after hooks. The `shared_routines` section
appears at the very end (it will grow as the shared library grows).
`routine_source` appears between `default_routine` and `hooks`.

Example config.yml:

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

### Error variant (src/error.rs)

Add `RoutineDisabled(String)` to `DecreeError`. This is returned when a
routine exists on disk but is not enabled in the registry.

### Routine resolution (src/routine.rs)

#### Directory layering

Update `find_routine_script` to accept the shared directory as an
optional second search path:

```rust
pub fn find_routine_script(
    routines_dir: &Path,
    shared_dir: Option<&Path>,
    name: &str,
    config: Option<&AppConfig>,
) -> Result<PathBuf, DecreeError>
```

Resolution order:

1. Check if the routine is enabled via `config.is_routine_enabled(name)`.
   Return `RoutineDisabled` if not.
2. Look in `routines_dir` (project-local `.decree/routines/`) for
   `<name>.sh` or `<name>`.
3. If not found and `shared_dir` is provided, look there.
4. If not found anywhere, return `RoutineNotFound`.

This means a project-local file always wins over a shared file with the
same name. To customize a shared routine, copy it into `.decree/routines/`
and modify it.

Update internal callers:

- `routine_detail()` — pass shared dir from config.
- `run_precheck()` — pass shared dir from config.

#### Prompt layering

Apply the same layering to prompt resolution. When `routine_source` is set
in config, derive the shared prompts path as the sibling `prompts/`
directory: if `routine_source` is `~/.decree/routines`, the shared prompts
path is `~/.decree/prompts`.

Update prompt template resolution in `src/commands/prompt.rs` to check
`.decree/prompts/` first, then the shared prompts directory.

### Routine listing (src/message.rs)

Change the signature of `list_routines` to accept config and shared dir:

```rust
pub fn list_routines(
    project_root: &Path,
    config: Option<&AppConfig>,
) -> Result<Vec<RoutineInfo>, DecreeError>
```

The function should:

1. Scan `.decree/routines/` for all `.sh` files (existing behavior).
2. If `config.routine_source` is set, also scan the shared directory.
3. Deduplicate by name — project-local wins over shared.
4. If config has `routines` and/or `shared_routines` sections, filter to
   only include routines where `config.is_routine_enabled(name)` returns
   true.
5. When config has no `routines` section (`None`), return all project-local
   routines (backward compat). Shared routines always require explicit
   enablement.

Update the internal call in `select_routine()` to pass config.

### Routine discovery

Discovery scans both directories and registers any new routines into
config. It runs at these checkpoints:

1. **`decree init`** — register routines created during init.
2. **`decree process`** — discover once at start, before beforeAll hook.
3. **`decree daemon`** — discover once at startup.

Discovery logic (new function `discover_routines` in
`src/commands/routine_sync.rs` or `src/routine.rs`):

```rust
pub fn discover_routines(
    project_root: &Path,
    config: &mut AppConfig,
) -> Result<bool, DecreeError>
```

Returns `true` if config was modified (caller should write it back).

Steps:

1. Scan `.decree/routines/*.sh` for project-local routines.
2. For each routine not in `config.routines`: add with `enabled: true`.
3. For each routine in `config.routines` whose file no longer exists in
   `.decree/routines/` AND is not present in the shared directory: set
   `deprecated: true`.
4. For each deprecated routine whose file has reappeared: clear
   `deprecated`.
5. If `routine_source` is set, scan the shared directory.
6. For each shared routine not in `config.shared_routines`: add with
   `enabled: false`.
7. For each routine in `config.shared_routines` whose file no longer
   exists in the shared directory: set `deprecated: true`.
8. For each deprecated shared routine whose file has reappeared: clear
   `deprecated`.
9. If any changes were made, write updated config back to
   `.decree/config.yml`.

Discovery must preserve existing entries — it only adds new ones and
updates deprecated status. It never removes entries or changes `enabled`
on existing entries.

#### Config writing

Discovery needs to update config.yml without destroying comments or
reordering fields. Two approaches:

- **Preferred**: Read config via serde, modify in-memory, write back the
  `routines` and `shared_routines` sections only using targeted YAML
  manipulation. The rest of the file is preserved as-is.
- **Simpler fallback**: Read the entire file, deserialize, modify, and
  re-serialize. This loses comments but is simpler to implement. Since
  `generate_config()` already uses manual string building, comments can
  be re-added by the generator.

Use whichever approach is more practical for the codebase. If comments
are important to preserve, use targeted section replacement.

### Init (src/commands/init.rs)

Update `generate_config()` to append a `routines` section after hooks.
Register all routines created during init:

- Always: `develop` and `rust-develop` with `enabled: true`.
- If git hooks enabled: `git-baseline` and `git-stash-changes` with
  `enabled: true`.

Emit commented-out `routine_source` and empty `shared_routines` as hints:

```yaml
# routine_source: "~/.decree/routines"
```

If `routine_source` would resolve to a directory that exists and contains
`.sh` files, run discovery to populate `shared_routines` with
`enabled: false` entries. This gives the user a visible menu of what's
available to enable.

### Command file updates

**src/commands/routine.rs** — In `run()`, load config before calling
`list_routines()` and pass it. In `verify()`, same pattern.

**src/commands/prompt.rs** — In `build_routines_text()`, load config and
pass to `list_routines()`. Update prompt template resolution to support
layered directories.

**src/commands/process.rs** — Run discovery at start (before beforeAll
hook). Pass config to `find_routine_script()` with shared dir. Pass
config to `list_routines()`.

**src/commands/daemon.rs** — Run discovery at startup. Pass config to
`find_routine_script()` with shared dir.

**src/hooks.rs** — Pass `None` for config to `find_routine_script()`.
Hooks bypass the registry — they are infrastructure, not user-facing
workflow routines. A hook routine must exist on disk but does not need
to be "enabled" in the registry.

### CLI: `decree routine-sync` command

Add a `RoutineSync` variant to the CLI in `src/cli.rs`:

```rust
/// Discover and register routines from project and shared directories
RoutineSync {
    /// Path to shared routines directory (overrides config)
    #[arg(long)]
    source: Option<String>,
},
```

This runs discovery manually — useful for seeing what's available after
adding routines to the shared library, without waiting for the next
process/daemon cycle.

`--source` overrides `config.routine_source`. If neither is set, only
project-local discovery runs.

Output should list what was found and what changed:

```
Project routines (.decree/routines/):
  develop         enabled
  rust-develop    enabled

Shared routines (~/.decree/routines/):
  deploy          enabled
  notify          disabled (new)
  data-pipeline   disabled
```

### Docker integration

#### docker/entrypoint.sh

Remove the `routine-sync.sh` call and the `compgen` check. The daemon
handles discovery automatically via `routine_source` in config.

#### docker/docker-compose.example.yml

Mount the shared routines directory:

```yaml
volumes:
  - .:/work
  - ~/.decree/routines:~/.decree/routines
```

#### Dockerfile

Remove the `yq` installation step. Remove the `routine-sync.sh` COPY
and `chmod`. The decree binary handles everything natively.

### Help text (src/templates/help.txt)

Add `routine-sync` to the command list:

```
  routine-sync  Discover and register routines
```

Add a section:

```
ROUTINE SYNC

  decree routine-sync [--source <dir>]

  Discover routines in .decree/routines/ and the shared routines
  directory, registering new ones in config.yml.

  Project-local routines default to enabled. Shared routines default
  to disabled — enable them in config.yml under shared_routines.

  Discovery also runs automatically at the start of process and daemon.

SHARED ROUTINES

  Set routine_source in config.yml to use a shared routines library:

    routine_source: "~/.decree/routines"

  The standard path is ~/.decree/routines. This works identically on
  bare metal and in Docker (mount ~/.decree/routines:~/.decree/routines).

  Resolution order:
  1. .decree/routines/ (project-local, wins if present)
  2. ~/.decree/routines/ (shared fallback)

  To customize a shared routine for your project, copy it into
  .decree/routines/ and modify it there. The local version takes
  precedence.

  Shared prompts work the same way: ~/.decree/prompts/ is the fallback
  for .decree/prompts/.
```

## Files to Create

- `src/commands/routine_sync.rs` — discovery logic and CLI command

## Files to Modify

- `src/cli.rs` — add RoutineSync command variant
- `src/commands/mod.rs` — add routine_sync module
- `src/main.rs` — wire up RoutineSync command
- `src/config.rs` — RoutineConfig, routine_source, routines, shared_routines
- `src/error.rs` — RoutineDisabled variant
- `src/message.rs` — list_routines with layered scanning and config filtering
- `src/routine.rs` — find_routine_script with shared dir fallback
- `src/commands/init.rs` — generate_config with routines section, discovery
- `src/commands/routine.rs` — pass config to list_routines
- `src/commands/prompt.rs` — pass config to list_routines, layered prompt resolution
- `src/commands/process.rs` — run discovery at start, pass config+shared dir
- `src/commands/daemon.rs` — run discovery at startup, pass config+shared dir
- `src/hooks.rs` — pass None to find_routine_script
- `src/templates/help.txt` — add routine-sync and shared routines docs
- `docker/entrypoint.sh` — remove routine-sync.sh call
- `docker/docker-compose.example.yml` — mount ~/.decree/routines
- `docker/README.md` — document shared routines convention
- `Dockerfile` — remove yq installation, remove routine-sync.sh COPY
- `.dockerignore` — remove docker/test-routine-sync.sh entry
- `tests/integration_test.rs` — add routine registry and discovery tests

## Files to Delete

- `docker/routine-sync.sh` — replaced by layered resolution + discovery
- `docker/test-routine-sync.sh` — bash tests replaced by Rust integration tests

## Acceptance Criteria

### Config deserialization

- **Given** a config.yml with `routines` and `shared_routines` sections
  **When** the config is loaded
  **Then** both are deserialized as `Some(map)` with correct states

- **Given** a config.yml without a `routines` section (legacy format)
  **When** the config is loaded
  **Then** `AppConfig.routines` is `None` (backward compat)

- **Given** a config.yml with `routine_source: "~/.decree/routines"`
  **When** `resolve_routine_source()` is called
  **Then** it returns the expanded absolute path

### Routine enablement

- **Given** config with `routines` section where `develop` is enabled and
  `rust-develop` is disabled
  **When** `is_routine_enabled("develop")` is called
  **Then** it returns `true`

- **Given** the same config
  **When** `is_routine_enabled("rust-develop")` is called
  **Then** it returns `false`

- **Given** config with no `routines` section (None)
  **When** `is_routine_enabled("anything")` is called for a project-local
  routine
  **Then** it returns `true` (backward compat)

- **Given** config with `shared_routines` where `deploy` is enabled
  **When** `is_routine_enabled("deploy")` is called
  **Then** it returns `true`

- **Given** config with `shared_routines` where `notify` is disabled
  **When** `is_routine_enabled("notify")` is called
  **Then** it returns `false`

- **Given** a routine not in either section when sections exist
  **When** `is_routine_enabled("unknown")` is called
  **Then** it returns `false`

- **Given** config with a routine marked `deprecated: true`
  **When** `is_routine_enabled` is called for that routine
  **Then** it returns `false` regardless of the `enabled` field

### Routine resolution (layered directories)

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
  **Then** `RoutineDisabled` is returned

- **Given** `routine_source` is not set
  **When** routines are resolved
  **Then** only `.decree/routines/` is searched

### Discovery

- **Given** a new `audit.sh` is added to `.decree/routines/`
  **When** `decree process` runs (discovery triggers)
  **Then** `audit` is added to `routines` in config with `enabled: true`

- **Given** a new `deploy.sh` is added to `~/.decree/routines/`
  **When** `decree process` runs (discovery triggers)
  **Then** `deploy` is added to `shared_routines` with `enabled: false`

- **Given** `deploy.sh` is removed from the shared directory
  **When** discovery runs
  **Then** `deploy` in `shared_routines` is marked `deprecated: true`

- **Given** a deprecated routine whose file reappears
  **When** discovery runs
  **Then** the `deprecated` flag is cleared

- **Given** an existing routine entry with `enabled: true`
  **When** discovery runs
  **Then** the `enabled` field is not changed (discovery never modifies
  existing enabled/disabled state)

- **Given** discovery finds no changes
  **When** it completes
  **Then** config.yml is not rewritten

### Init

- **Given** `decree init` is run
  **When** the config.yml is generated
  **Then** the `routines` section lists `develop` and `rust-develop` as
  `enabled: true`, and a commented-out `routine_source` hint is present

- **Given** `decree init` is run with git hooks enabled
  **When** the config.yml is generated
  **Then** `git-baseline` and `git-stash-changes` also appear in
  `routines` as `enabled: true`

- **Given** `~/.decree/routines/` exists with `.sh` files during init
  **When** `decree init` is run
  **Then** the `shared_routines` section is populated with those routines
  as `enabled: false`

### Interactive routine command

- **Given** config with one routine disabled
  **When** `decree routine` lists available routines
  **Then** the disabled routine does not appear

### AI router

- **Given** config with routines and shared_routines sections
  **When** the AI router builds the routine selection prompt
  **Then** only enabled routines from both sections appear

### decree routine-sync command

- **Given** `.decree/routines/` has `develop.sh` and `~/.decree/routines/`
  has `deploy.sh` and `notify.sh`
  **When** `decree routine-sync` is run
  **Then** output lists all routines with their status, `deploy` and
  `notify` are added to `shared_routines` as `enabled: false` if new

- **Given** `routine_source` is not set and `--source` is not passed
  **When** `decree routine-sync` is run
  **Then** only project-local discovery runs (no error)

- **Given** `--source ~/other-routines` is passed
  **When** `decree routine-sync` runs
  **Then** the specified directory is used instead of `routine_source`

### Prompt layering

- **Given** `routine_source: "~/.decree/routines"` and a prompt template
  `migration.md` exists in both `.decree/prompts/` and `~/.decree/prompts/`
  **When** the prompt is resolved
  **Then** the project-local version is used

- **Given** a prompt template exists only in `~/.decree/prompts/`
  **When** the prompt is resolved
  **Then** the shared version is used

### Hooks

- **Given** a hook routine (e.g., git-baseline) that is not in the
  routines registry
  **When** `decree process` runs and the hook is triggered
  **Then** the hook executes normally (hooks bypass the registry)

### Docker integration

- **Given** the Docker compose with `~/.decree/routines:~/.decree/routines`
  mounted and config with `routine_source: "~/.decree/routines"`
  **When** the daemon starts
  **Then** discovery registers shared routines and the daemon uses layered
  resolution (same behavior as bare metal)

- **Given** the Docker image
  **When** it is built
  **Then** `yq` is not installed and `routine-sync.sh` is not present

### Backward compatibility

- **Given** an existing project with config.yml that has no `routines` key
  and no `routine_source`
  **When** `decree process`, `decree routine`, or `decree verify` is run
  **Then** all project-local filesystem routines are available (identical
  to pre-migration behavior)

- **Given** an existing project with no `routine_source` configured
  **When** discovery runs
  **Then** only project-local routines are discovered (no shared dir
  scanned)

### Tests

- **Given** the test suite
  **When** `cargo test` is run
  **Then** all existing tests pass, plus new tests for: config
  deserialization with routines/shared_routines/routine_source, tilde
  expansion, is_routine_enabled across both sections, layered
  find_routine_script, list_routines with shared dir, discovery adding
  new project routines as enabled, discovery adding new shared routines
  as disabled, deprecation detection, init routines section generation,
  prompt layering, and backward compatibility with no routines section
