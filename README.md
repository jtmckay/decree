# Decree

Specification-driven project execution framework. Specs go in, tested implementations come out, every change is checkpointed and reversible.

Decree is a self-contained Rust CLI that turns structured specification files into executed, verified code changes via AI-powered routines. It brings CI/CD rigor to AI-assisted development — with automatic retries, dead-lettering, change tracking, and optional scheduled execution.

## How It Works

```
specs/           .decree/routines/       .decree/runs/
01-auth.spec.md  ──→  develop.sh     ──→  changes.diff
02-db.spec.md         rust-develop.sh     pre/post checkpoints
03-logs.spec.md       *.ipynb (opt)       stdout/stderr logs
```

1. You write **spec files** describing work to be done
2. `decree process` feeds each spec to a **routine** (shell script or Jupyter notebook) that delegates to AI, builds, and tests
3. Every execution is **checkpointed** — full file manifests before and after, unified diffs of all changes
4. Failures **retry** up to 3 times (partial work preserved on early retries, clean revert with failure context on final attempt), then dead-letter
5. You **review** diffs and **selectively apply** changes to your working tree

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

## AI Providers

Decree supports three AI backends, selected during `init`:

- **Claude CLI** — `claude -p {prompt}`
- **GitHub Copilot CLI** — `copilot -p {prompt}`
- **Embedded** — Qwen 2.5 1.5B-Instruct via llama.cpp (~1.1 GB download, no account required)

The embedded model handles routing and interactive chat. External CLIs handle heavier implementation tasks. Mix and match via `.decree/config.yml`.

## Key Concepts

**Specs** are immutable markdown files with optional YAML frontmatter. Once processed, they're never modified — corrections require new specs. See [sow.md](sow.md) for the full scope and design rationale.

**Routines** are executable templates (`.sh` or `.ipynb`) in `.decree/routines/` that receive spec content and AI commands as parameters. Shell scripts take precedence over notebooks.

**Messages** are the unit of execution. Specs become messages, `decree run` creates messages, cron jobs create messages. Each message gets its own run directory with checkpoints, diffs, and logs.

**Chains** let routines spawn follow-up messages (depth-limited to 10), enabling multi-step workflows within a single execution.

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
