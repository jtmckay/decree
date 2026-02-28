# Decree

AI orchestrator for structured, reproducible workflows. Write specs, let AI execute them, review the diffs.

Decree is a self-contained Rust CLI that orchestrates AI agents through a message-driven pipeline. You describe work as spec files, Decree routes each one to the right AI-powered routine, checkpoints your repo before and after, retries on failure, and collects the results for your review. Think CI/CD for AI-assisted development.

## How It Works

```
specs/           .decree/routines/       .decree/runs/
01-auth.spec.md  ──→  develop.sh     ──→  changes.diff
02-db.spec.md         rust-develop.sh     pre/post checkpoints
03-logs.spec.md       *.ipynb (opt)       stdout/stderr logs
```

1. You write **spec files** describing work to be done
2. Decree **routes** each spec to an AI-powered **routine** (shell script or Jupyter notebook) that implements, builds, and tests the change
3. Every execution is **checkpointed** — full file manifests before and after, unified diffs of all changes
4. Failures **retry** up to 3 times (partial work preserved on early retries, clean revert with failure context on final attempt), then dead-letter
5. You **review** diffs and **selectively apply** changes to your working tree
6. Routines can **chain** follow-up messages, enabling multi-step AI workflows within a single execution

## Commands

| Command                  | Purpose                                                                              |
| ------------------------ | ------------------------------------------------------------------------------------ |
| `decree init`            | Scaffold `.decree/` with routines, templates, config, and optional model download    |
| `decree plan [template]` | Interactive AI planning session → produces spec files                                |
| `decree process`         | Batch-process all unprocessed specs in order                                         |
| `decree run -p "prompt"` | Execute a single ad hoc task immediately                                             |
| `decree sow`             | Generate a Statement of Work from your specs using the relative .decree/plans/sow.md |
| `decree diff [id]`       | Review unified diffs from any execution                                              |
| `decree apply [id]`      | Selectively re-apply changes with conflict detection                                 |
| `decree daemon`          | Background service: monitors inbox + fires cron jobs                                 |
| `decree ai`              | Embedded LLM REPL with session persistence                                           |
| `decree status`          | Spec processing progress and recent history                                          |
| `decree log [id]`        | Inspect execution logs                                                               |
| `decree bench`           | Benchmark embedded model performance                                                 |

## Quick Start

```bash
cargo install --path .
decree init        # choose AI provider, configure routines
decree plan spec   # interactively generate spec files
decree process     # execute all specs
decree diff        # review what changed
decree apply --all # apply changes to working tree
```

## AI Backends

Decree orchestrates any AI that can run from the command line. Three backends are supported out of the box, selected during `init`:

- **Claude CLI** — `claude -p {prompt}`
- **GitHub Copilot CLI** — `copilot -p {prompt}`
- **Embedded** — Qwen 2.5 1.5B-Instruct via llama.cpp (~1.1 GB download, no account required)

The embedded model handles routing and interactive planning. External CLIs handle heavier implementation work. Mix and match per command slot via `.decree/config.yml`.

## Key Concepts

**Specs** are immutable markdown files with optional YAML frontmatter. They're the input to the orchestrator — each spec describes a unit of work for AI to execute. Once processed, they're never modified; corrections require new specs.

**Routines** are the AI execution templates (`.sh` or `.ipynb`) in `.decree/routines/`. Each routine defines how an AI agent should handle a task — what tools to invoke, what checks to run, what constitutes success. Decree routes specs to routines automatically or via frontmatter.

**Messages** are the unit of orchestration. Specs become messages, `decree run` creates messages, cron jobs create messages. Each message gets its own isolated run directory with checkpoints, diffs, and logs.

**Chains** let routines spawn follow-up messages (depth-limited to 10), enabling multi-step agentic workflows — one AI action can trigger the next.

## Project Structure

```
.decree/
├── config.yml       # AI providers, retry limits, default routine
├── routines/        # Shell scripts and/or Jupyter notebooks
├── plans/           # Templates for planning sessions
├── cron/            # Scheduled job definitions
├── inbox/           # Pending messages (done/ and dead/ subdirs)
├── runs/            # Execution history with checkpoints and diffs
└── sessions/        # Saved AI conversation histories
specs/
├── *.spec.md        # Your specification files
└── processed-spec.md # Tracking file
```

## Examples

The `examples/` directory contains self-contained projects demonstrating different Decree patterns:

- **[historical-portraits](examples/historical-portraits/)** — Chain-based pipeline. Each spec triggers a 3-step chain (research → prompt-craft → generate) to produce AI art portraits of historical figures via ComfyUI. Demonstrates message chaining, custom parameters, and inter-step data sharing.

- **[business-eval](examples/business-eval/)** — Chain-based analysis pipeline. Each spec is a different business idea that triggers a 4-step chain (market analysis → competitive landscape → financial model → executive summary). Demonstrates accumulated context through chains and multiple independent businesses processed in sequence.

## GPU Acceleration

```bash
cargo build --release --features vulkan  # Vulkan
cargo build --release --features cuda    # NVIDIA CUDA
cargo build --release --features metal   # Apple Metal
```

## Requirements

- Bash (Linux/macOS built-in, Git Bash or WSL on Windows)
- Python 3 only if using Jupyter notebook routines (opt-in)
- No cloud account needed for core functionality
