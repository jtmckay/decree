---
routine: rust-develop
---

# 15: Docker Fixes

## Overview

Fix three Docker issues: GHA cache backend requires buildx, `.dockerignore`
excludes files needed by the build, glibc mismatch between build and runtime
stages, runtime AI tool selection, and TTY warning in podman-compose.

## Requirements

### GitHub Actions workflow (.github/workflows/docker.yml)

Add a `docker/setup-buildx-action@v3` step **after** the login step and
**before** the metadata step. The `docker/build-push-action` with GHA
caching (`cache-from: type=gha` / `cache-to: type=gha`) requires the
Buildx builder driver, which is not available by default.

### .dockerignore

Replace the blanket `docker/` exclusion with specific file exclusions.
The build needs `docker/entrypoint.sh` and `docker/routine-sync.sh` in the
context (they are `COPY`ed in the Dockerfile). Exclude only:

- `docker/README.md`
- `docker/docker-compose.example.yml`
- `docker/test-routine-sync.sh`

Keep all other existing exclusions unchanged.

### Dockerfile — glibc mismatch

Change the build stage base from `rust:1-slim` to `rust:1-slim-bookworm`.
The runtime stage uses `node:24-bookworm-slim` (Debian Bookworm, glibc 2.36).
The default `rust:1-slim` is based on Debian Trixie (glibc 2.40+), so
binaries compiled there fail at runtime with
`GLIBC_2.39 not found (required by decree)`.

### Dockerfile — runtime AI tool selection

Remove the `RUN npm i -g opencode-ai` layer from the Dockerfile. This is
the largest contributor to the ~1GB image size.

Remove the `/root/.config/opencode` volume declaration. Change volumes to
just `/work` and `/routines`.

### Entrypoint — DECREE_AI environment variable

Add AI tool installation to `docker/entrypoint.sh`, after the
`DECREE_CONTAINER` validation and before `decree init`. Check the
`DECREE_AI` environment variable and install the selected tool if not
already present on PATH:

- `opencode` — `npm i -g opencode-ai`
- `claude` — `npm i -g @anthropic-ai/claude-code`
- `copilot` — Install the GitHub CLI (`gh`) from the official `.deb`
  release if not present, then install the copilot extension via
  `gh extension install github/gh-copilot`.

If `DECREE_AI` is empty or unset, skip installation (no default).
If set to an unknown value, print a warning to stderr listing the
supported values.

Use `command -v <tool>` checks to skip installation on subsequent
container restarts.

### docker-compose.example.yml

Update to reflect changes:

- Add `DECREE_AI=opencode` (or claude, copilot) to the environment section
  with a comment listing the options.
- Remove the opencode config volume mount.
- Add `tty: false` to suppress the podman-compose TTY warning:
  `could not start menu, an error occurred while starting: open /dev/tty`.

### docker/README.md

Update documentation to reflect:

- Image no longer bundles opencode; AI tool is selected at runtime via
  `DECREE_AI`.
- Add `DECREE_AI` to the environment variables table.
- Remove `/root/.config/opencode` from the volumes table.
- Update the quick start example to include `-e DECREE_AI=opencode`.
- Update troubleshooting: replace "API key not mounted" with
  "AI tool not installed" guidance.

## Files to Modify

- `.github/workflows/docker.yml` — add buildx setup step
- `.dockerignore` — replace `docker/` with specific exclusions
- `Dockerfile` — fix base image, remove opencode install, update volumes
- `docker/entrypoint.sh` — add DECREE_AI installation logic
- `docker/docker-compose.example.yml` — add DECREE_AI, tty: false, remove opencode volume
- `docker/README.md` — update for AI selection

## Acceptance Criteria

- **Given** the GitHub Actions workflow with buildx setup
  **When** a push to main triggers the workflow
  **Then** the build completes without "Cache export is not supported for
  the docker driver" error

- **Given** the updated `.dockerignore`
  **When** `docker build .` runs
  **Then** `docker/entrypoint.sh` and `docker/routine-sync.sh` are found
  in the build context

- **Given** the Dockerfile with `rust:1-slim-bookworm` builder
  **When** the image is built and started
  **Then** the decree binary runs without glibc version errors

- **Given** `DECREE_AI=opencode` in the environment
  **When** the container starts
  **Then** opencode is installed via npm and available on PATH

- **Given** `DECREE_AI=claude` in the environment
  **When** the container starts
  **Then** claude-code is installed via npm and available on PATH

- **Given** `DECREE_AI` is unset
  **When** the container starts
  **Then** no AI tool is installed and the container starts normally

- **Given** the docker-compose example with `tty: false`
  **When** `docker compose up` or `podman-compose up` runs
  **Then** no "could not start menu" TTY warning appears
