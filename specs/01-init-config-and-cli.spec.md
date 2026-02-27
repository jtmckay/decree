---
routine: develop
---

# 01: Init, Config, and CLI

## Overview

The scaffolding everything else plugs into. `decree init` creates the project
layout, selects AI providers, and optionally downloads the embedded model.
The config schema, full CLI definitions, main dispatch, error types, and
utility commands (`decree sow`, `decree status`, `decree log`) all live here.
Default file contents (plan templates, routine templates, gitignore) are
specified in spec 02.

## Requirements

### Directory Creation

`decree init` must create:

```
~/.decree/
└── models/                   # GGUF model files (shared across projects)
.decree/
├── config.yml
├── .gitignore
├── routines/
│   ├── develop.sh           # Default routine (always created)
│   ├── develop.ipynb        # Default notebook routine (only if notebook support enabled)
│   ├── rust-develop.sh      # Rust routine (always created)
│   └── rust-develop.ipynb   # Rust notebook routine (only if notebook support enabled)
├── plans/
│   ├── sow.md               # SOW template
│   └── spec.md              # Spec template
├── cron/                     # Scheduled jobs (cron expressions)
├── inbox/                    # Message queue for execution
│   ├── done/                # Successfully processed messages
│   └── dead/                # Failed messages (exhausted retries)
├── runs/                     # Flat message directories (gitignored)
├── sessions/                 # AI REPL session files (gitignored)
└── venv/                     # Python venv (only if notebook support enabled)
specs/                        # Adjacent to .decree/, at project root
└── processed-spec.md         # Empty tracker file
```

### Message ID Format

A message ID has the form `<chain>-<seq>`:

- **Chain ID**: `YYYYMMDDHHmmss` + 2-digit counter, e.g. `2025022514320000`.
  The counter is `00` by default; if another chain starts in the same
  second, the counter increments (`01`, `02`, etc.). This looks like a
  timestamp with centisecond precision but the last 2 digits are a
  collision counter. Generated once for the root message — all messages
  in the same chain share this chain ID.
- **Sequence number**: starts at `0` for the root message and increments
  by 1 for each subsequent message in the chain. The sequence tracks
  processing order — it is a simple counter.

Examples: `2025022514320000-0`, `2025022514320000-1`, `2025022514320000-2`.

Commands accept either a full ID (`2025022514320000-2`) to target a single
message, or just the chain ID (`2025022514320000`) to target the entire chain.
Unique prefixes are also accepted.

### Message Directory Structure

Every processed message gets its own directory under `.decree/runs/`:

```
.decree/runs/
├── 2025022514320000-0/              # Root: spec 01-add-auth
│   ├── message.md               # Copy of the inbox message
│   ├── manifest.json            # Pre-execution tree state (checkpoint)
│   ├── changes.diff             # What this execution changed
│   ├── routine.log              # Shell script stdout/stderr (for .sh routines)
│   ├── output.ipynb             # Papermill output notebook (for .ipynb routines)
│   └── papermill.log            # Papermill stderr (for .ipynb routines)
├── 2025022514320000-1/              # Follow-up spawned by -0
│   └── ...
└── 2025022514321500-0/              # New chain
    └── ...
```

### AI Provider Selection

Two interactive selection prompts (arrow-key navigation with fuzzy
filtering per the Interactive Selection UX convention):

**Planning AI** — which AI handles `decree plan`:

- Claude CLI (default)
- GitHub Copilot CLI
- Embedded (decree ai) — local GGUF model

**Router AI** — which AI selects routines for messages:

- Embedded (decree ai) (default)
- Claude CLI
- GitHub Copilot CLI

### Automatic GGUF Download

After provider selection, if model file is missing at the configured
`ai.model_path` (default `~/.decree/models/qwen2.5-1.5b-instruct-q5_k_m.gguf`):

- Prompt the user: "Model not found. Download Qwen 2.5 1.5B-Instruct Q5_K_M (~1.1 GB)? [Y/n]"
- On confirmation, download using a built-in HTTP client (Rust crate, e.g.
  `ureq` or `reqwest`) — no dependency on `wget`, `curl`, or any external tool
- Create `~/.decree/models/` if needed
- Show a progress indicator during download (bytes received / total size)
- Print the HuggingFace URL so the user can download manually if they decline

The download is built into the decree binary itself. The CLI must be fully
self-contained — no reliance on external download tools.

### Notebook Support (Optional)

After GGUF download, prompt:

```
Enable Jupyter Notebook routine support? (requires Python 3) [y/N]
```

Default is **No**. When enabled:

- `develop.ipynb` is created alongside `develop.sh` in `.decree/routines/`
- `.decree/venv/` is created lazily on first notebook execution (with
  papermill + ipykernel)
- Notebook routines (`.ipynb`) take precedence over shell scripts when both
  exist for the same routine name (see spec 06 for discovery rules)
- `config.yml` records `notebook_support: true`

When disabled (the default):

- Only `develop.sh` is created in `.decree/routines/`
- No `.decree/venv/` is ever created
- `.ipynb` files in `.decree/routines/` are ignored by the processor
- `config.yml` records `notebook_support: false`

This keeps the default install free of Python dependencies. Users can
enable notebook support later by setting `notebook_support: true` in
`config.yml` and running `decree init` again to generate the notebook
template.

### Platform Requirements

The decree binary is self-contained (Rust, embedded LLM). The only
external runtime requirement is **bash**, needed for shell script routines:

| Platform    | bash availability                                                            |
| ----------- | ---------------------------------------------------------------------------- |
| **Linux**   | Built-in (`/bin/bash`)                                                       |
| **macOS**   | Built-in (`/bin/bash`, v3.2+)                                                |
| **Windows** | Requires [Git for Windows](https://gitforwindows.org/) (bundles bash) or WSL |

The AI CLI invoked by routines (e.g. `claude`, `copilot`) is a separate
user-provided dependency — not bundled with decree.

### Config File (`config.yml`)

```yaml
ai:
  model_path: "~/.decree/models/qwen2.5-1.5b-instruct-q5_k_m.gguf"
  n_gpu_layers: 0

commands:
  # Claude CLI (default):
  planning: "claude -p {prompt}"
  planning_continue: "claude --continue"

  # GitHub Copilot:
  # planning: "copilot -p {prompt}"
  # planning_continue: "copilot --continue"

  # Embedded (in-process — no external commands needed):
  # planning: "decree ai"
  # planning_continue: ""

  router: "decree ai"

max_retries: 3 # Per-message retry limit
max_depth: 10 # Inbox recursion limit
default_routine: develop # Fallback routine name
notebook_support: false # Enable .ipynb routines (requires Python 3)
```

Fields:

- `ai.model_path` — path to GGUF file (default: `~/.decree/models/qwen2.5-1.5b-instruct-q5_k_m.gguf`)
- `ai.n_gpu_layers` — GPU offload setting
- `commands.planning` — planning AI command (prompt injection via `{prompt}`)
- `commands.planning_continue` — planning AI continue command (interactive handoff)
- `commands.router` — router/internal AI command
- `max_retries` — per-message retry limit (default: 3)
- `max_depth` — inbox recursion limit (default: 10)
- `default_routine` — fallback routine name (default: "develop")
- `notebook_support` — enable `.ipynb` routine support (default: false)

The default `model_path` always points to
`~/.decree/models/qwen2.5-1.5b-instruct-q5_k_m.gguf` regardless of which
AI providers are selected for planning/router. The embedded model is always
available via `decree ai` and `decree bench`.

### Interactive Selection UX

All interactive prompts that present a list of choices (AI provider,
notebook support, routine selection, plan selection) use **arrow-key
navigation** with **fuzzy type-ahead filtering**:

- Up/Down arrows move the highlight
- Typing narrows the list by fuzzy-matching against item labels
- Enter confirms the highlighted selection
- The config default (if any) is pre-highlighted

This applies to every selection prompt in the CLI — `decree init`,
`decree run` (interactive mode), and `decree plan`. Use a Rust TUI crate
that provides this out of the box (e.g. `dialoguer`, `inquire`, or
similar). Y/N confirmation prompts remain simple inline prompts.

### Full CLI

```
decree                                         # Smart default
decree init [--model-path PATH]                # Initialize project

decree plan [PLAN]                             # Interactive planning
decree run [-m NAME] [-p PROMPT] [-v K=V ...]   # Run a message
decree run                                     # Interactive mode (TTY only)
decree process                                 # Batch: all unprocessed specs
decree daemon [--interval SECS]                # Daemon: monitor inbox + cron

decree diff                                    # Latest message's changes
decree diff <ID>                               # Specific message or chain
decree diff --since <ID>                       # From message onward

decree apply                                   # List available messages
decree apply <ID>                              # Apply message or chain
decree apply --through <ID>                    # All messages up through ID
decree apply --since <ID>                      # All messages from ID onward
decree apply --all                             # Apply everything
decree apply <ID> --force                      # Skip conflict check

decree sow                                     # Derive SOW from specs
decree ai [-p PROMPT] [--json] [--max-tokens N] # New AI session
decree ai --resume [SESSION_ID]                # Resume AI session
decree bench [PROMPT] [--runs N] [--max-tokens N] [--ctx N] # Benchmark
decree status                                  # Show progress
decree log [ID]                                # Show execution log
```

`ID` is a message ID (`<chain>-<seq>`), a chain ID (`<chain>`), or any
unique prefix of either.

### Main Dispatch

`main.rs` must:

- Check for `.decree/` existence (except for `init`)
- Dispatch to all command handlers
- Handle `decree daemon` daemon lifecycle (signal handling)

### Error Types

`DecreeError` variants for the full command set:

- `SpecNotFound` — referenced spec doesn't exist
- `RoutineNotFound` — referenced routine doesn't exist
- `MaxRetriesExhausted` — all retries for a message failed
- `MaxDepthExceeded` — inbox recursion limit hit
- `NoSpecs` — no spec files found in specs/
- `MessageNotFound` — referenced message ID doesn't exist in runs/

### `decree sow`

Generates a Statement of Work from the project's specs using the SOW
template as the prompt.

1. Read `.decree/plans/sow.md` (the SOW template from spec 02)
2. Build a prompt that includes:
   - The full SOW template content as formatting/structural guidance
   - The `specs/` directory path so the AI can read all spec files
3. Send the prompt to `commands.planning` (the configured planning AI)
4. Write the AI's output to `sow.md` at the project root

The SOW template teaches the AI the structure (Business Context, Jobs to
Be Done, User Scenarios, Scope, Deliverables, Acceptance Criteria,
Assumptions & Constraints). The AI reads the specs to understand what has
been built or is planned, then synthesizes a coherent SOW that captures
the business intent behind the full body of work.

### `decree status`

Summarizes progress: processed specs, pending inbox messages, and recent
message history.

### `decree log`

Reads execution artifacts from message directories. Which artifacts exist
depends on the routine format:

- **Shell script routines**: `routine.log` (combined stdout/stderr)
- **Notebook routines**: `output.ipynb` (executed notebook with cell
  outputs) and `papermill.log` (execution stderr)

Usage:

- Without an ID: shows the most recent message's log
- With a chain ID: shows all logs in the chain
- Ambiguous prefixes list matching candidates

### Python Venv

Only relevant when `notebook_support: true`. Create `.decree/venv/` with
papermill + ipykernel installed. The venv is created lazily on first
notebook routine execution, not during `decree init`. When
`notebook_support: false` (the default), no venv is ever created and
`.ipynb` routines are ignored. Respect `DECREE_VENV` env var for shared
venvs in tests.

### `.gitignore`

`.decree/.gitignore` includes `venv/`, `inbox/`, `runs/`, and `sessions/`.

## Acceptance Criteria

- **Given** a project with no `.decree/` directory
  **When** the user runs `decree init`
  **Then** `.decree/` is created with subdirectories: `routines/`, `plans/`,
  `cron/`, `inbox/` (with `done/`), `runs/`, `sessions/`
  **And** `specs/` is created at the project root with an empty `processed-spec.md`

- **Given** `decree init` is running
  **When** the user is prompted for AI provider
  **Then** an arrow-key selector with fuzzy filtering shows three choices
  (Claude CLI, Copilot CLI, Embedded)
  **And** separate selectors appear for planning AI and router AI

- **Given** `decree init` is running
  **When** the user is prompted for notebook support
  **Then** the default is No
  **And** the user can opt in to enable `.ipynb` routine support

- **Given** the user has selected AI providers
  **When** init completes
  **Then** `config.yml` is written with the selected `commands.planning`,
  `commands.planning_continue`, `commands.router`, and `notebook_support`

- **Given** no GGUF model file exists at the configured path
  **When** `decree init` finishes provider selection
  **Then** it offers to download the model using the built-in HTTP client
  **And** downloads to `~/.decree/models/` with a progress indicator on confirmation
  **And** prints the manual download URL if the user declines

- **Given** the CLI is invoked
  **When** any valid subcommand is passed (`init`, `plan`, `run`, `process`, `daemon`, `diff`, `apply`, `sow`, `ai`, `bench`, `status`, `log`)
  **Then** the command is accepted and dispatched to the correct handler

- **Given** spec files exist in `specs/` and `.decree/plans/sow.md` exists
  **When** the user runs `decree sow`
  **Then** the planning AI receives a prompt containing the SOW template
  and a reference to the `specs/` directory
  **And** a coherent `sow.md` is written to the project root following
  the template structure

- **Given** message directories exist in `.decree/runs/`
  **When** the user runs `decree log` without a message ID
  **Then** the most recent message's execution log is displayed

- **Given** the user passes a message ID prefix to `decree log`
  **When** the prefix uniquely identifies a message
  **Then** that message's execution log is displayed

- **Given** an ambiguous message ID prefix is provided
  **When** any command tries to resolve it
  **Then** the matching candidates are listed and the user is asked to be more specific

- **Given** `decree status` is invoked
  **When** specs and messages exist
  **Then** progress is summarized: processed specs, pending inbox messages, recent message history

- **Given** a new error scenario occurs
  **When** the error is raised
  **Then** the corresponding `DecreeError` variant is used
