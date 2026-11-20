---
routine: rust-develop
---

# 14: Docker

## Overview

Package decree as a Docker image with opencode bundled, publish to GitHub
Container Registry (ghcr.io/jtmckay/decree), and provide a reference
docker-compose snippet so other projects can add decree as a service.

## Requirements

### Dockerfile

Create `Dockerfile` at the project root. Use a multi-stage build with
dependency caching.

**Build stage** (`rust:1-slim`):

1. Copy `Cargo.toml` and `Cargo.lock` first, create a dummy `src/main.rs`,
   and run `cargo build --release` to cache dependencies in a separate
   layer.
2. Copy the real source tree and `cargo build --release` again (only
   recompiles decree itself on code changes).

**Runtime stage** (`node:24-bookworm-slim`):

- Install runtime deps: `bash`, `git`, `curl`, `ca-certificates`, `flock`
  (from `util-linux`, usually pre-installed).
- Install [`yq`](https://github.com/mikefarah/yq) (single binary) for
  YAML parsing/writing in `routine-sync.sh`.
- Node.js 24 LTS and npm are provided by the base image.
  Run `npm i -g opencode-ai`.
- Copy the decree binary from the build stage to `/usr/local/bin/decree`.
- Copy `docker/entrypoint.sh` and `docker/routine-sync.sh` to
  `/usr/local/bin/`, `chmod +x` both.
- Set `WORKDIR /work`.
- Declare volumes: `/work`, `/routines`, `/root/.config/opencode`.
- Set `ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]`.

### Entrypoint script

Create `docker/entrypoint.sh`. This is a thin orchestrator — all routine
sync logic lives in `routine-sync.sh`.

1. Validate `DECREE_CONTAINER`: must be non-empty, contain only
   `[a-zA-Z0-9_-]` (no path separators, no `__`, no whitespace). Fail
   with a clear error if invalid.
2. Run `decree init --no-color </dev/null` if `/work/.decree` does not
   exist. The `</dev/null` ensures non-interactive mode. `--no-color`
   prevents escape codes in logs.
3. Run `routine-sync.sh` (see below). Skip if `/routines` contains no
   `.sh` files.
4. If `$@` (CMD arguments) were passed, exec them directly:
   `exec "$@"`. This allows `docker run decree decree process`.
5. If `DECREE_DAEMON=true` (default), exec
   `decree daemon --no-color --interval "${DECREE_INTERVAL:-2}"`.
6. Otherwise, `exec bash` for interactive use.

### routine-sync.sh

Create `docker/routine-sync.sh`. This script manages bidirectional routine
synchronization between `/routines` (shared host directory) and
`/work/.decree/routines/` (container project). It is independently testable
and must be covered by `docker/test-routine-sync.sh`.

All reads/writes to `routines.yml` use `yq` (mikefarah/yq) for reliable
YAML manipulation. All writes must be wrapped in `flock /routines/.lock`
to prevent concurrent container races.

#### DECREE_CONTAINER validation

The script must reject container IDs containing `__` (would break variant
parsing), `/` or `\` (path separators), or whitespace. Only
`[a-zA-Z0-9_-]` are allowed, and the value must not contain a double
underscore sequence. The entrypoint validates this before calling
routine-sync.sh, but routine-sync.sh should also guard against it.

#### Configuration file: `/routines/routines.yml`

Managed automatically by routine-sync.sh. Format:

```yaml
routines:
  deploy:
    file: deploy.sh
    variants:
      decree_1: deploy__decree_1.sh
    ignore: []
  notify:
    file: notify.sh
    variants: {}
    ignore: []
  develop:
    file: develop.sh
    deprecated: true # base file was removed from /routines
    variants:
      decree_2: develop__decree_2.sh
    ignore:
      - decree_3
```

Fields per routine entry:

- `file` — base `.sh` filename in `/routines/`.
- `variants` — map of `<container_id>: <variant_filename>`. Variant files
  use `__` as separator: `<name>__<container>.sh`.
- `ignore` — list of container IDs that skip this routine (project-local
  version exists). Defaults to empty.
- `deprecated` — `true` if the base `.sh` file has been removed from
  `/routines/`. Set automatically during auto-discovery. When a user
  removes a deprecated entry from `routines.yml`, the corresponding routine
  is deleted from all containers on their next startup (see cleanup step).

#### Startup sync sequence

routine-sync.sh runs these steps in order:

**Step 1 — Auto-discover** (under flock):

1. Scan `/routines/*.sh`, excluding variant files (those containing `__`)
   and `routines.yml` itself.
2. For each base `.sh` file not yet in `routines.yml`: add a new entry with
   empty variants, empty ignore, no deprecated flag.
3. For each entry in `routines.yml` whose base file no longer exists in
   `/routines/`: set `deprecated: true`. (Do not remove the entry — the
   user decides when to delete it.)
4. For each entry marked `deprecated` whose base file has reappeared: remove
   the `deprecated` flag.

**Step 2 — Sync-out** (under flock): For each non-deprecated routine in
`routines.yml` where this container is NOT in the `ignore` list:

1. If `.decree/routines/<name>.sh` does not exist, skip.
2. Determine the comparison source: if a variant for this container exists
   in `routines.yml`, use `/routines/<name>__<container>.sh`. Otherwise use
   the base `/routines/<name>.sh`.
3. Compare the local `.decree/routines/<name>.sh` against the source
   (`sha256sum` comparison).
4. If they differ: copy the local file to
   `/routines/<name>__<container>.sh`, `chmod +x` it, and add/update the
   variant entry in `routines.yml`.

**Step 3 — Sync-in**: For each non-deprecated routine in `routines.yml`:

1. If this container is in the `ignore` list, skip.
2. Determine the source: if a variant for this container exists, use
   `/routines/<name>__<container>.sh`. Otherwise use the base
   `/routines/<name>.sh`.
3. If `.decree/routines/<name>.sh` exists, this routine has **no** variant
   for this container, and the local file does NOT match the base
   (first encounter with a project-local routine):
   - Add this container to the `ignore` list (under flock).
   - Skip.
4. Copy the source to `.decree/routines/<name>.sh` and `chmod +x` it.

**Step 4 — Cleanup**: For each entry that was in `routines.yml` at the
start of this run but is no longer present (user manually removed it):

1. Delete `.decree/routines/<name>.sh` from the container if it exists.
2. This allows users to fully remove a routine by deleting the base file
   AND removing the entry from `routines.yml`.

To detect removed entries: routine-sync.sh snapshots the list of routine
names from `routines.yml` at the start, then compares after reload. Any
name present in the snapshot but absent after reload was removed by the
user (or another container) during this startup.

#### Variant naming

```
<name>__<container>.sh
```

Examples: `deploy__decree_1.sh`, `notify__abc123def456.sh`

The `__` (double underscore) separator is unambiguous: routine names must
not contain `__`, and `DECREE_CONTAINER` is validated to reject `__`.

#### Container identity for compose

Each service should set a stable, unique `DECREE_CONTAINER`. Recommended:
service name with a numeric suffix.

```yaml
services:
  decree_1:
    image: ghcr.io/jtmckay/decree:latest
    environment:
      - DECREE_CONTAINER=decree_1
  decree_2:
    image: ghcr.io/jtmckay/decree:latest
    environment:
      - DECREE_CONTAINER=decree_2
```

Docker's default hostname is a random 12-char hex ID, which changes on
every `docker run`. Setting `DECREE_CONTAINER` explicitly ensures stable
variant filenames across restarts.

### test-routine-sync.sh

Create `docker/test-routine-sync.sh`. This is a standalone test harness
for `routine-sync.sh` that exercises edge cases using temporary directories
(no Docker required). It should test:

1. **First run, empty state**: no `routines.yml`, `.sh` files in
   `/routines` — creates `routines.yml`, copies routines in.
2. **Auto-discovery**: new `.sh` file added between runs — entry appears.
3. **Sync-out**: modify a routine locally, re-run — variant created.
4. **Sync-in with variant**: variant exists, re-run — variant used as
   source.
5. **Variant update**: modify again after variant exists — variant updated
   in place.
6. **Project-local ignore**: routine exists locally before shared — added
   to ignore list, not overwritten.
7. **Deprecation**: remove base `.sh` — entry marked `deprecated: true`.
8. **Un-deprecation**: restore base `.sh` — deprecated flag removed.
9. **Cleanup on entry removal**: remove entry from `routines.yml`, re-run —
   routine deleted from container.
10. **Concurrent safety**: two syncs with flock — no corruption.
11. **Permissions**: copied files have `+x`.
12. **Invalid DECREE_CONTAINER**: values with `__`, `/`, whitespace — fails
    with error.
13. **External base update**: base file updated, no local modification —
    new base copied in on sync-in.
14. **Multi-container isolation**: two different container IDs, one modifies
    — only that container gets the variant.

Each test should set up a temp directory structure, run routine-sync.sh,
and assert the expected state. Exit non-zero on any failure.

### Environment variables

| Variable           | Default     | Purpose                                 |
| ------------------ | ----------- | --------------------------------------- |
| `DECREE_DAEMON`    | `true`      | Start in daemon mode when `true`        |
| `DECREE_INTERVAL`  | `2`         | Daemon polling interval in seconds      |
| `DECREE_CONTAINER` | `$HOSTNAME` | Container identity for routine variants |

`DECREE_CONTAINER` defaults to `$HOSTNAME`, which Docker sets to the short
container ID (a unique 12-character hex string). Users should override it
with a stable, human-readable name for predictable variant filenames. Must
match `[a-zA-Z0-9_-]+` and must not contain `__`.

### Volume mount points

Declare three volumes in the Dockerfile:

| Mount                    | Purpose                                                     |
| ------------------------ | ----------------------------------------------------------- |
| `/work`                  | Project working directory (primary). `.decree/` lives here. |
| `/routines`              | Shared/global routines (optional)                           |
| `/root/.config/opencode` | Opencode config + API keys                                  |

### docker-compose.yml (example)

Create `docker/docker-compose.example.yml` showing how a consuming project
would add decree as a service:

```yaml
services:
  decree:
    image: ghcr.io/jtmckay/decree:latest
    volumes:
      - .:/work
      - ${HOME}/.config/opencode:/root/.config/opencode
      - ~/shared-routines:/routines # optional: shared routines
    environment:
      - DECREE_DAEMON=true
      - DECREE_INTERVAL=2
      - DECREE_CONTAINER=decree_1
    restart: unless-stopped
```

### GitHub Actions workflow

Create `.github/workflows/docker.yml`:

- Trigger on push to `main` and on version tags (`v*`).
- Set `permissions: packages: write` on the job (required for GHCR push).
- Use `docker/login-action` to authenticate with GHCR using
  `${{ github.actor }}` and `${{ secrets.GITHUB_TOKEN }}`.
- Use `docker/metadata-action` to generate tags: `latest` on main,
  semver on version tags.
- Use `docker/build-push-action` with `push: true` and `cache-from`/
  `cache-to` using GitHub Actions cache for layer reuse.

### Docker publishing README

Create `docker/README.md` with:

1. **Overview** — what the image contains (decree + opencode).
2. **Quick start** — `docker run` one-liner.
3. **docker-compose** — reference to `docker-compose.example.yml`.
4. **Environment variables** — table of `DECREE_DAEMON`, `DECREE_INTERVAL`,
   `DECREE_CONTAINER`.
5. **Volumes** — table of mount points and their purpose.
6. **Shared routines** — explanation of `/routines` mount, `routines.yml`,
   variants, and the sync lifecycle.
7. **Publishing** — step-by-step instructions:
   - Prerequisites: push access to the repo, `packages: write` permission.
   - Automatic: push to `main` or tag `vX.Y.Z` — CI builds and publishes.
   - Manual: `docker build -t ghcr.io/jtmckay/decree:latest .` then
     `docker push ghcr.io/jtmckay/decree:latest`. Requires
     `echo $GITHUB_TOKEN | docker login ghcr.io -u USERNAME --password-stdin`.
8. **Troubleshooting** — common issues (API key not mounted, init prompting,
   routine not appearing).

### .dockerignore

Create `.dockerignore` to exclude `target/`, `.decree/`, `.git/`, and other
build artifacts from the Docker context.

## Files to Create

- `Dockerfile` — multi-stage build with dependency caching
- `docker/entrypoint.sh` — init + sync + daemon/shell orchestrator
- `docker/routine-sync.sh` — shared routine bidirectional sync logic
- `docker/test-routine-sync.sh` — test harness for routine-sync.sh
- `docker/docker-compose.example.yml` — reference compose file
- `docker/README.md` — publishing and usage documentation
- `.github/workflows/docker.yml` — CI publish workflow
- `.dockerignore` — build context exclusions

## Acceptance Criteria

### Image build

- **Given** the Dockerfile exists
  **When** `docker build -t decree .` is run
  **Then** the image builds successfully with both `decree` and `opencode`
  available on PATH

- **Given** only source files in `src/` change (not `Cargo.toml`)
  **When** `docker build` is run
  **Then** the dependency layer is cached and only decree recompiles

### Container lifecycle

- **Given** a built decree image and an empty directory mounted at `/work`
  **When** the container starts with default environment
  **Then** `decree init --no-color` runs non-interactively (no prompts, no
  escape codes), `.decree/` is created in `/work`, and the daemon starts

- **Given** a built decree image and a directory with existing `.decree/`
  mounted at `/work`
  **When** the container starts
  **Then** `decree init` is skipped and the daemon starts against the
  existing configuration

- **Given** `DECREE_DAEMON=false` in the environment
  **When** the container starts
  **Then** the container drops into an interactive shell instead of starting
  the daemon

- **Given** `DECREE_INTERVAL=10` in the environment
  **When** the container starts in daemon mode
  **Then** `decree daemon --interval 10` is invoked

- **Given** CMD arguments are passed (e.g., `docker run decree decree process`)
  **When** the container starts
  **Then** the arguments are exec'd directly, bypassing daemon/shell logic

- **Given** the container is running in daemon mode
  **When** the container receives SIGTERM (e.g., `docker stop`)
  **Then** the decree daemon shuts down gracefully

### DECREE_CONTAINER validation

- **Given** `DECREE_CONTAINER` is set to `my__bad`
  **When** the container starts
  **Then** the entrypoint exits with a clear error message

- **Given** `DECREE_CONTAINER` is set to `my/bad` or `my bad`
  **When** the container starts
  **Then** the entrypoint exits with a clear error message

- **Given** `DECREE_CONTAINER` is not set
  **When** the container starts
  **Then** it defaults to `$HOSTNAME` (Docker's 12-char container ID)

### Routine sync — auto-discovery

- **Given** `/routines` contains `deploy.sh` and `notify.sh` with no
  `routines.yml`
  **When** routine-sync.sh runs
  **Then** `routines.yml` is created with entries for both, both are copied
  into `.decree/routines/` with `+x` permission

- **Given** a new file `audit.sh` is added to `/routines` between restarts
  **When** routine-sync.sh runs
  **Then** `routines.yml` gains an `audit` entry and the file is copied in

- **Given** `deploy.sh` is deleted from `/routines` but the entry remains
  in `routines.yml`
  **When** routine-sync.sh runs
  **Then** the `deploy` entry is marked `deprecated: true`

- **Given** a `deploy` entry is marked `deprecated: true` and `deploy.sh`
  is restored to `/routines`
  **When** routine-sync.sh runs
  **Then** the `deprecated` flag is removed

### Routine sync — sync-out

- **Given** a shared routine `deploy.sh` was synced into the container
  **When** the routine is modified locally and routine-sync.sh runs
  **Then** the modified version is written to
  `/routines/deploy__<container>.sh` and the variant is recorded in
  `routines.yml`

- **Given** a variant `deploy__<container>.sh` already exists
  **When** the routine is modified again and routine-sync.sh runs
  **Then** the existing variant file is updated in place

- **Given** two containers (A and B) share `/routines` with base `deploy.sh`
  **When** container A modifies `deploy.sh` and runs routine-sync.sh
  **Then** `/routines/deploy__A.sh` is created, container B still uses the
  base `deploy.sh`

### Routine sync — sync-in

- **Given** `/routines` contains both `deploy.sh` (base) and
  `deploy__<container>.sh` (variant for this container)
  **When** routine-sync.sh runs
  **Then** the variant is used as the source, not the base

- **Given** the base `deploy.sh` in `/routines` is updated externally
  and the container has not modified its copy
  **When** routine-sync.sh runs
  **Then** the updated base is copied into the container, replacing the
  old version

- **Given** `/routines` is mounted with `develop.sh` and the project
  already has a project-local `develop.sh` (first encounter, no variant)
  **When** routine-sync.sh runs
  **Then** the local `develop.sh` is preserved, the container is added to
  the `ignore` list in `routines.yml`

- **Given** routines are copied into `.decree/routines/`
  **When** the copy completes
  **Then** each copied file has executable permission (`chmod +x`)

### Routine sync — cleanup

- **Given** a routine entry exists in `routines.yml` when sync starts
  **When** the entry is removed from `routines.yml` (by user or another
  container) before sync-in runs
  **Then** `.decree/routines/<name>.sh` is deleted from the container

### Routine sync — concurrency

- **Given** two containers start simultaneously against the same `/routines`
  **When** both run routine-sync.sh concurrently
  **Then** `routines.yml` is not corrupted (flock serializes writes)

### Routine sync — tests

- **Given** `docker/test-routine-sync.sh` exists
  **When** it is executed (no Docker required)
  **Then** all test cases pass, covering: first run, auto-discovery,
  sync-out, sync-in with variant, variant update, project-local ignore,
  deprecation, un-deprecation, cleanup on entry removal, permissions,
  invalid container ID, external base update, and multi-container isolation

### Compose and publishing

- **Given** the docker-compose example file
  **When** a consuming project copies it and runs `docker compose up`
  **Then** the decree service starts with the host project directory mounted
  at `/work`

- **Given** `/routines` is not mounted (directory is empty)
  **When** the container starts
  **Then** the entrypoint skips routine sync entirely with no errors

- **Given** a push to `main` or a version tag
  **When** the GitHub Actions workflow runs
  **Then** the image is built and pushed to `ghcr.io/jtmckay/decree` with
  appropriate tags (the workflow has `packages: write` permission)

- **Given** the `.dockerignore` file exists
  **When** `docker build` runs
  **Then** `target/`, `.decree/`, and `.git/` are excluded from the build
  context

- **Given** `docker/README.md` exists
  **When** a developer reads it
  **Then** it contains quick start, compose reference, environment variables,
  volumes, shared routines explanation, and step-by-step publishing
  instructions (both CI and manual)
