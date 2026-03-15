# Decree

An AI orchestrator for spec-driven development. Write specs, run `decree process`, get working code.

## Why Decree

AI coding assistants are powerful but ad hoc. You prompt, you review, you prompt again. Nothing is repeatable, nothing is tracked, and multi-step workflows need constant babysitting.

Decree treats AI work like database migrations. You write spec files describing what you want built. Routines define _how_ AI processes each spec — implement, build, test, fix. Processing runs them in order, one at a time, each building on the last. Everything is logged.

The result: you focus on _what_ to build. The routines handle _how_.

## Quick Start

```bash
cargo install decree
decree init          # scaffold project, pick your AI tool
```

This creates `.decree/` with routines, prompts, config, and a router.

## Prompts

Interactive prompt templates live in `.decree/prompts/`. They inject project context — processed migrations, available routines, config — so your AI conversations start informed:

```bash
decree prompt sow    # plan a new statement of work for a new project
decree prompt routine      # get help writing a new routine (flow)
decree prompt migration    # plan next batch of specs
```

## Workflow example: Spec-Driven Development

**1. Write specs**

Create migration files in `.decree/migrations/`, numbered for ordering:

```
.decree/migrations/
├── 01-auth-system.spec.md
├── 02-user-profiles.spec.md
└── 03-api-endpoints.spec.md
```

Each spec is markdown with optional YAML frontmatter:

```markdown
---
routine: develop
---

# Auth System

Implement email/password authentication with session tokens.

## Requirements

- POST /auth/register creates a user
- POST /auth/login returns a session token
- Sessions expire after 24 hours

## Acceptance Criteria

- Registration with duplicate email returns 409
- Invalid credentials return 401
- Expired tokens are rejected
```

**2. Process**

```bash
decree process
```

Each spec is processed in order through the assigned routine. The default `develop` routine invokes your AI tool twice — once to implement, once to verify acceptance criteria. Failed specs retry with prior attempt logs as context.

**3. Review**

```bash
decree status        # see what's been processed
decree log 01        # see execution output for a spec
```

## Blackbox Testing with Specs

Specs work well as blackbox test cases. Define inputs and expected outputs. The routine implements code to satisfy them. You never describe _how_ — only _what_.

Write specs around observable behavior:

```markdown
# Markdown Parser

Parse markdown to HTML.

## Acceptance Criteria

- `# Hello` produces `<h1>Hello</h1>`
- `**bold**` produces `<strong>bold</strong>`
- Empty input produces empty output
- Nested lists render correctly
```

The AI figures out the implementation. The acceptance criteria are the tests.

## Routines

Routines are shell scripts in `.decree/routines/` that define how work gets done. They receive the spec as a message file and call your AI tool directly.

The default `develop` routine:

1. Sends the spec to your AI tool for implementation
2. Sends it again for verification against acceptance criteria

The `rust-develop` routine adds build and test steps:

1. AI implements the spec
2. `cargo build --release && cargo test`
3. AI reads build/test output and fixes failures

Write your own routines for any workflow — linting passes, documentation generation, image creation, data pipelines. A routine is just a bash script that calls whatever tools you need.

```bash
decree routine       # list available routines
decree routine-sync  # sync routine registry with filesystem
decree verify        # check all routine pre-checks pass
```

## Shared Routines

Build a library of routines and share them across projects. Set `routine_source` in config to point at a shared directory:

```yaml
routine_source: "~/.decree/routines"
```

Project-local routines in `.decree/routines/` take precedence. Shared routines are fallbacks. The same layering applies to prompts.

Routines are tracked in a registry in `config.yml`. Discovery runs automatically at `decree init`, `decree process`, and `decree daemon` — or manually with `decree routine-sync`:

```yaml
routines:
  develop:
    enabled: true
  rust-develop:
    enabled: true

shared_routines:
  deploy:
    enabled: true
  notify:
    enabled: false
```

New project-local routines default to enabled. New shared routines default to disabled. Routines whose files disappear are marked deprecated. Hooks bypass the registry — they only need the script to exist on disk.

## Chaining

Routines can write follow-up messages to `.decree/outbox/`. Decree processes them depth-first before moving to the next migration. This enables multi-step pipelines:

```
market-analysis → competitive-landscape → financial-model → executive-summary
```

One spec in, four documents out.

## AI Tool Permissions

Your AI tool needs permission to read and write files in the repo. Configure this per-project so routines can operate non-interactively.

For Claude Code, create `.claude/settings.local.json`:

```json
{
  "permissions": {
    "allow": [
      "Bash(cargo build:*)",
      "Bash(cargo test:*)",
      "Read",
      "Write",
      "Edit",
      "Glob",
      "Grep"
    ]
  }
}
```

Other tools have similar mechanisms — check your AI tool's docs for non-interactive / headless permissions.

## Lifecycle Hooks

Configure hooks in `.decree/config.yml` for cross-cutting concerns:

```yaml
hooks:
  beforeEach: git-baseline
  afterEach: git-stash-changes
```

The built-in git hooks stash a baseline before each spec and checkpoint changes after. Failed specs restore to baseline before retrying. Every attempt is preserved as a named stash.

## Daemon & Cron

For recurring work, run the daemon:

```bash
decree daemon
```

It polls `.decree/cron/` for scheduled messages and `.decree/inbox/` for new work. Cron messages use standard cron syntax in frontmatter:

```markdown
---
cron: "0 9 * * 1-5"
routine: daily-review
---

Run the morning code review.
```

## Docker

Run decree in a container with no local install. The Docker image installs your AI tool on startup:

```yaml
services:
  decree:
    image: ghcr.io/jtmckay/decree:latest
    volumes:
      - .:/work
    environment:
      - DECREE_AI=opencode  # opencode, claude, or copilot
      - DECREE_DAEMON=true
      - DECREE_INTERVAL=2
    restart: unless-stopped
```

Mount a shared routine library:

```yaml
    volumes:
      - .:/work
      - ~/.decree/routines:/routines
```

See `examples/docker/` for a working setup.

## Project Structure

```
.decree/
├── config.yml          # AI tool config, retries, hooks, routine registry
├── router.md           # instructions for automatic routine selection
├── processed.md        # tracks completed migrations
├── migrations/         # spec files (your input)
├── routines/           # shell scripts (your workflows)
├── prompts/            # interactive prompt templates
├── cron/               # scheduled messages
├── inbox/              # messages being processed
├── outbox/             # follow-up messages from routines
├── runs/               # execution logs (the audit trail)
└── dead/               # exhausted messages for review
```
