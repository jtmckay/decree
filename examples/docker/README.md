# Docker — Containerized Decree

Run decree in a Docker container with on-demand AI tool installation and
optional shared routine libraries.

## What This Demonstrates

- **Docker deployment** — decree runs headless in a container, no local
  install required
- **On-demand AI tools** — set `DECREE_AI` to install opencode, claude, or
  copilot at startup
- **Shared routine volumes** — mount a host directory of routines into the
  container so multiple projects share the same library
- **Daemon mode** — container runs `decree daemon` by default, polling for
  new migrations and cron jobs

## Usage

```bash
cd examples/docker
docker compose up
```

This starts a decree container that:
1. Installs the AI tool specified by `DECREE_AI`
2. Runs `decree init` if `.decree/` doesn't exist
3. Starts `decree daemon` polling every 2 seconds

Drop migration files into `.decree/migrations/` and they'll be processed
automatically.

## docker-compose.yml

```yaml
services:
  decree:
    image: ghcr.io/jtmckay/decree:latest
    volumes:
      - .:/work
    environment:
      - DECREE_AI=opencode
      - DECREE_DAEMON=true
      - DECREE_INTERVAL=2
      - DECREE_CONTAINER=decree_docker_example
    tty: false
    restart: unless-stopped
```

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `DECREE_AI` | (none) | AI tool to install: `opencode`, `claude`, or `copilot` |
| `DECREE_DAEMON` | `true` | `true` runs daemon; `false` drops to bash shell |
| `DECREE_INTERVAL` | `2` | Daemon polling interval in seconds |
| `DECREE_CONTAINER` | hostname | Container name for git stash labels |

## Shared Routines

Mount a shared routine directory to reuse routines across projects:

```yaml
services:
  decree:
    image: ghcr.io/jtmckay/decree:latest
    volumes:
      - .:/work
      - ~/my-routines:/routines
    environment:
      - DECREE_AI=claude
```

Then in your project's `.decree/config.yml`:

```yaml
routine_source: /routines

shared_routines:
  my-shared-routine:
    enabled: true
```

## Interactive Shell

To drop into a shell instead of running the daemon:

```bash
docker compose run -e DECREE_DAEMON=false decree
```

Or pass a specific command:

```bash
docker compose run decree decree process --no-color
```
