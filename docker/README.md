# Decree Docker Image

## Overview

Pre-built Docker image containing **decree** (AI orchestrator) and **opencode** (AI coding tool). Runs as a daemon that watches for migrations and processes them automatically.

## Quick Start

```bash
docker run --rm -v "$(pwd):/work" \
  -v "$HOME/.config/opencode:/root/.config/opencode" \
  ghcr.io/jtmckay/decree:latest
```

## docker-compose

See [`docker-compose.example.yml`](docker-compose.example.yml) for a reference configuration:

```bash
cp docker/docker-compose.example.yml docker-compose.yml
# Edit volumes/environment as needed
docker compose up -d
```

## Environment Variables

| Variable           | Default     | Purpose                                 |
| ------------------ | ----------- | --------------------------------------- |
| `DECREE_DAEMON`    | `true`      | Start in daemon mode when `true`        |
| `DECREE_INTERVAL`  | `2`         | Daemon polling interval in seconds      |
| `DECREE_CONTAINER` | `$HOSTNAME` | Container identity for routine variants |

`DECREE_CONTAINER` must match `[a-zA-Z0-9_-]+` and must not contain `__`. Docker sets `$HOSTNAME` to a random 12-char hex ID by default — override it for stable, human-readable variant filenames.

## Volumes

| Mount                    | Purpose                                                     |
| ------------------------ | ----------------------------------------------------------- |
| `/work`                  | Project working directory (primary). `.decree/` lives here. |
| `/routines`              | Shared/global routines (optional)                           |
| `/root/.config/opencode` | Opencode config + API keys                                  |

## Shared Routines

Mount a host directory at `/routines` to share routine scripts across containers.

### How it works

1. **Auto-discovery**: On startup, `.sh` files in `/routines` are registered in `routines.yml`.
2. **Sync-in**: Registered routines are copied into each container's `.decree/routines/`.
3. **Sync-out**: If a container modifies a routine, the change is saved as a **variant** (`<name>__<container>.sh`) so other containers aren't affected.
4. **Project-local**: If a routine already exists locally when first encountered, the container is added to an `ignore` list — the local version is preserved.

### routines.yml

Managed automatically. Format:

```yaml
routines:
  deploy:
    file: deploy.sh
    variants:
      decree_1: deploy__decree_1.sh
    ignore: []
```

- `file` — base script filename
- `variants` — per-container overrides (auto-created on modification)
- `ignore` — containers that keep their own local version
- `deprecated` — set when the base file is removed from `/routines`

### Multi-container setup

```yaml
services:
  decree_1:
    image: ghcr.io/jtmckay/decree:latest
    environment:
      - DECREE_CONTAINER=decree_1
    volumes:
      - .:/work
      - ~/shared-routines:/routines
  decree_2:
    image: ghcr.io/jtmckay/decree:latest
    environment:
      - DECREE_CONTAINER=decree_2
    volumes:
      - ./other-project:/work
      - ~/shared-routines:/routines
```

## Publishing

### Prerequisites

- Push access to `github.com/jtmckay/decree`
- `packages: write` permission (automatic for repo collaborators)

### Automatic (CI)

Push to `main` or tag `vX.Y.Z` — the GitHub Actions workflow builds and pushes to `ghcr.io/jtmckay/decree` with appropriate tags.

### Manual

```bash
echo "$GITHUB_TOKEN" | docker login ghcr.io -u USERNAME --password-stdin
docker build -t ghcr.io/jtmckay/decree:latest .
docker push ghcr.io/jtmckay/decree:latest
```

## Troubleshooting

**API key not mounted**: If opencode fails with auth errors, ensure your config is mounted at `/root/.config/opencode`.

**`decree init` prompts for input**: The entrypoint passes `</dev/null` and `--no-color` to ensure non-interactive mode. If you see prompts, check that your decree version supports `--no-color`.

**Routine not appearing**: Check that:
- The `.sh` file is in `/routines` (not a subdirectory)
- The filename doesn't contain `__` (reserved for variants)
- The container isn't in the routine's `ignore` list in `routines.yml`
- The routine isn't marked `deprecated` in `routines.yml`

**Variant filenames changing on restart**: Set `DECREE_CONTAINER` to a stable value. Without it, Docker's random hostname changes every `docker run`.
