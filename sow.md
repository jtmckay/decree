# SOW: Decree — AI-Orchestrated Structured Development Workflow

## Business Context

Software teams increasingly use AI assistants for implementation, but the
interaction model remains ad hoc: developers manually prompt AIs, copy results,
run builds, fix errors, and repeat. There is no structured pipeline that
captures intent, delegates execution, tracks changes, retries on failure, and
schedules recurring work — all without requiring developers to babysit each
step.

Decree fills this gap as a self-contained CLI that turns structured
specifications into executed, verified, and checkpointed code changes. It
brings the rigor of CI/CD pipelines to AI-assisted development: specs go in,
tested implementations come out, and every change is tracked with full
reversibility. The tool runs anywhere bash runs, embeds its own small language
model for routing and interactive use, and requires no cloud AI account for
core functionality.

## Jobs to Be Done

1. When I have a body of work to plan, I want to collaborate with an AI in an
   interactive session pre-loaded with my project context, so I can produce
   well-structured specs without manually assembling prompts.

2. When I have spec files describing work to be done, I want to run a single
   command that processes them in order — delegating to AI, building, testing,
   and retrying on failure — so I can step away while implementation happens.

3. When a routine fails, I want the system to retry intelligently (leaving
   partial work for the AI to learn from, then reverting to a clean slate with
   failure context on the final attempt), so transient issues are resolved
   without my intervention.

4. When I need to review what an execution changed, I want to see standard
   unified diffs and selectively re-apply changes, so I maintain full control
   over what lands in my codebase.

5. When I have recurring maintenance or development tasks, I want to schedule
   them with cron expressions so they execute automatically on a cadence
   without manual triggering.

6. When I initialize a new project, I want sensible defaults with the option
   to choose my AI provider, enable notebook support, and download an embedded
   model — so setup is a single command with no external dependencies beyond
   bash.

7. When I want quick AI assistance without an external service, I want an
   embedded LLM with an interactive REPL, session persistence, and context
   management, so I can have multi-turn conversations locally.

8. When I need to understand my project's progress, I want a derived Statement
   of Work synthesized from my specs, execution status, and log inspection, so
   I can communicate status without manually assembling reports.

## User Scenarios

- **Greenfield project setup**: A developer runs `decree init` in a new
  repository. They select Claude CLI for planning and the embedded model for
  routing, decline notebook support, and confirm the GGUF model download.
  Within a minute they have a `.decree/` directory with routines, templates,
  and configuration — ready to plan and execute work.

- **Spec-driven feature implementation**: A developer has written three spec
  files (`01-add-auth.spec.md`, `02-add-database.spec.md`,
  `03-add-logging.spec.md`). They run `decree process`. Each spec is
  processed in order: the routine delegates to an AI for implementation, runs
  verification, and on success marks the spec as processed. If `02` fails,
  the system retries up to three times, ultimately reverting to a clean state
  and dead-lettering the message before continuing to `03`.

- **Interactive planning session**: A developer runs `decree plan spec`. An
  AI session opens, pre-loaded with the spec template and existing project
  specs. The developer describes a new feature; the AI proposes numbered spec
  files. After approval, the AI writes them to `specs/`. The developer then
  runs `decree process` to execute them.

- **Ad hoc task execution**: A developer runs
  `decree run -p "Fix type errors in src/auth.rs" -v routine=develop`. A
  message is created, normalized, and immediately processed. The routine
  spawns a follow-up message to fix remaining issues, which is processed
  depth-first in the same chain.

- **Selective change review and application**: After processing, a developer
  runs `decree diff 2025022514320000` to review an entire chain's changes.
  Satisfied with the root message but not the follow-up, they run
  `decree apply 2025022514320000-0` to cherry-pick just the first message's
  changes onto a fresh branch.

- **Scheduled maintenance**: A developer creates a cron file
  `.decree/cron/nightly-tests.md` with `cron: "0 2 * * *"` and
  `routine: rust-develop`. They start `decree daemon`. Every night at 2 AM,
  the daemon creates an inbox message from the cron file and processes it
  through the full pipeline — checkpoint, execute, diff, disposition.

- **Embedded AI conversation**: A developer runs `decree ai` for a quick
  question. The embedded Qwen model loads, a session file is created, and
  they have a multi-turn conversation with context window management.
  Later, they resume the session with `decree ai --resume` and the full
  history is restored.

## Scope

**In scope:**

- Project initialization with AI provider selection, directory scaffolding,
  and embedded model download
- Default routine templates (general-purpose and Rust-specific) in both
  shell script and Jupyter notebook formats
- Embedded LLM (Qwen 2.5 1.5B-Instruct) for routing, interactive REPL
  with session persistence, and benchmarking
- Self-contained checkpoint system (manifest + unified diff) with no git
  dependency
- Message format specification, normalization pipeline, and AI-assisted
  routine selection
- Routine system with two formats (`.sh` and `.ipynb`), custom parameter
  injection, and discovery precedence rules
- Single-message execution (`decree run`), batch spec processing
  (`decree process`), and the core message processing loop with retry
  strategy and dead-lettering
- Daemon mode with inbox monitoring and cron-based scheduled execution
- Interactive AI planning sessions with project context injection and plan
  template selection
- Change review (`decree diff`) and selective re-application (`decree apply`)
  with conflict detection
- SOW generation, status reporting, and execution log inspection
- Cross-platform support (Linux, macOS, Windows via Git Bash/WSL)

**Out of scope (future work):**

- Remote/distributed execution or multi-machine coordination
- Web UI or graphical interface
- Cloud-hosted AI model serving (all AI is either embedded or user-provided CLI)
- Spec file editing or modification after processing (specs are immutable)
- Multi-user collaboration or access control
- Package distribution or installation management (decree is a single binary)

## Deliverables

1. **Project initialization and configuration** — `decree init` with
   interactive AI provider selection, GGUF download, notebook support toggle,
   and full directory/config scaffolding
2. **Default templates** — SOW and spec plan templates, general-purpose and
   Rust-specific routine templates in both shell script and notebook formats,
   embedded at compile time
3. **Embedded AI system** — Interactive REPL with session persistence and
   context management, one-shot mode, piped input support, and hardware-aware
   benchmarking
4. **Checkpoint system** — Pure-Rust manifest creation, unified diff
   generation, and verified revert capability with no external tool
   dependencies
5. **Message format and normalization** — Spec file processing, inbox message
   handling (fully structured, minimal, and bare formats), and deterministic
   field derivation with AI-assisted routine selection
6. **Routine system** — Dual-format execution (shell scripts via bash,
   notebooks via papermill), custom parameter discovery and injection,
   format-aware discovery precedence, and follow-up message spawning
7. **Run and process commands** — Single-message execution with interactive
   and non-interactive modes, batch spec processing, retry strategy with
   progressive context, and dead-letter queue
8. **Daemon and cron** — Continuous inbox monitoring, cron expression
   evaluation, scheduled message creation, and graceful shutdown
9. **Interactive planning** — AI sessions pre-loaded with project context,
   plan template selection, and seamless continuation across external and
   embedded AI providers
10. **Change review and application** — Diff preview by message, chain, or
    range; selective re-application with conflict detection; force-apply
    with confirmation; and message listing with diff statistics

## Acceptance Criteria

- A developer can run `decree init` and have a fully configured project with
  routines, templates, and configuration in under a minute
- `decree process` executes all unprocessed specs in order, with each spec
  independently checkpointed, retried on failure, and dead-lettered on
  exhaustion — without halting the batch
- The retry strategy leaves partial work in place for early retries (AI
  learns from mistakes) and reverts to a clean slate with failure context
  for the final attempt
- `decree run` works in both interactive mode (guided prompts with recall)
  and non-interactive mode (flags and piped input) with no user prompts
- `decree daemon` continuously processes inbox messages and fires cron jobs
  on schedule, with graceful shutdown preserving in-progress work
- `decree plan` opens an AI conversation pre-loaded with the selected
  template and project state, producing spec files or inbox messages
- `decree diff` and `decree apply` operate on individual messages, entire
  chains, or ranges — with conflict detection preventing partial application
- The embedded AI REPL maintains multi-turn sessions with persistent history,
  context window management, and resume capability
- The entire system works identically with or without a git repository —
  the checkpoint system is pure Rust with no external tool dependencies
- The decree binary is self-contained: embedded model for routing/REPL, all
  diffing/hashing/file-walking in Rust, only bash required at runtime

## Assumptions & Constraints

- The target platform has bash available (built-in on Linux/macOS, Git for
  Windows or WSL on Windows)
- External AI CLIs (Claude, Copilot) are user-provided and not bundled —
  decree only invokes them via configured command strings
- The embedded model (Qwen 2.5 1.5B-Instruct, ~1.1 GB) is sufficient for
  routing and interactive use but not for complex implementation tasks
- Spec files are immutable once processed — corrections require new specs
- Notebook support requires Python 3 and is opt-in; the default installation
  has no Python dependency
- The GGUF model path defaults to `~/.decree/models/` (shared across
  projects) while project state lives in `.decree/` (per-project)
- Message chains have a maximum depth of 10 to prevent unbounded recursion
- Per-message retry limit defaults to 3 attempts
