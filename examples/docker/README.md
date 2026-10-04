# Docker — Containerized Decree

Run decree in a Docker container with on-demand AI tool installation and
optional shared machines.

## What This Demonstrates

- **Docker deployment** — decree runs headless in a container, no local
  install required
- **On-demand AI tools** — set `DECREE_AI` to install opencode, claude, or
  copilot at startup
- **Shared machine volumes** — mount a host directory of machines and scripts
  into the container so multiple projects share the same library
- **Daemon mode** — container runs `decree daemon` by default, polling for
  new migrations, inbox messages and cron jobs

## The Project

`.decree/` holds one machine, `develop`
([`.decree/machines/develop.yml`](.decree/machines/develop.yml), drawn in
[`.decree/graph/develop.md`](.decree/graph/develop.md)): `precheck` checks
that opencode is installed, `implement` hands the message to opencode, and
`verify` has opencode check the acceptance criteria. The scripts are in
`.decree/scripts/`. The sample migration asks for a `hello.sh`.

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
automatically. Check the project from the host (or inside the container) with:

```bash
decree check
decree status
```

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
    restart: unless-stopped
```

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `DECREE_AI` | (none) | AI tool to install: `opencode`, `claude`, or `copilot` |
| `DECREE_DAEMON` | `true` | `true` runs daemon; `false` drops to bash shell |
| `DECREE_INTERVAL` | `2` | Daemon polling interval in seconds |

## Shared Machines

Mount a shared directory to reuse machines and scripts across projects:

```yaml
services:
  decree:
    image: ghcr.io/jtmckay/decree:latest
    volumes:
      - .:/work
      - ~/my-machines:/shared
    environment:
      - DECREE_AI=claude
```

decree has no shared source of its own: a project's machines load only from
`.decree/machines/`, and scripts resolve from `.decree/scripts/<machine>/`, then
`.decree/scripts/`. To use a shared machine or script, symlink it into those
directories, pointing at the path where the container sees it:

```bash
ln -s /shared/machines/review.yml .decree/machines/review.yml
ln -s /shared/scripts/notify.sh .decree/scripts/notify.sh
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
