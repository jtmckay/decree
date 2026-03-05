# SOW: AI-Orchestrated Development Workflows

## Business Context

Software teams increasingly use AI coding assistants, but each interaction is
ad hoc — prompts are improvised, outputs are untracked, and multi-step
workflows require constant human shepherding. There is no repeatable way to
define a body of work, feed it through AI-powered routines, and get reliable,
auditable results.

Decree solves this by treating AI work like database migrations: define the
work declaratively, process it through configurable routines, and track
everything that happened. Teams get structured, reproducible AI workflows
that scale from a single spec to an entire project built from ordered
migration files.

## Jobs to Be Done

1. When I have a body of work to accomplish, I want to break it into ordered
   migration files, so each piece is processed sequentially with full context
   from prior steps.

2. When I run a migration, I want it routed to the right AI routine
   automatically, so I don't manually invoke different tools for different
   tasks.

3. When a routine fails, I want automatic retries with prior attempt logs
   available as context, so transient failures resolve without my intervention.

4. When I need a multi-step workflow (research, then generate, then verify),
   I want routines to chain follow-up messages, so complex pipelines run
   end-to-end without manual orchestration.

5. When I want to track what AI actually did, I want every execution logged
   in run directories with message history, so I can audit, debug, and
   reproduce results.

6. When I need ongoing automated work, I want cron-scheduled messages
   processed by a daemon, so recurring tasks run without manual triggers.

7. When I'm starting a new project, I want guided setup that scaffolds
   routines, prompts, and config, so I'm productive immediately without
   learning the file layout by hand.

8. When I want to interact with AI directly, I want templated prompts with
   project context injected, so my conversations start with the right
   information already assembled.

## User Scenarios

- **Greenfield project build**: A developer runs `decree init`, selects their
  AI tool, then writes 10 migration specs describing a new CLI application.
  Running `decree process` executes each spec in order — the develop routine
  invokes the AI, which reads each spec, implements the code, and verifies
  acceptance criteria. The developer reviews results after each migration via
  `decree log` and `decree status`.

- **Multi-step analysis pipeline**: A business analyst creates specs for three
  startup ideas. Each spec routes to a `market-analysis` routine that chains
  to `competitive-landscape`, then `financial-model`, then
  `executive-summary`. One `decree process` command produces a complete
  evaluation for each business — four documents per idea, fully automated.

- **Creative asset generation**: An artist writes specs describing historical
  figures and art styles. Processing chains through research, prompt-crafting,
  and image generation routines. Each step passes artifacts to the next via
  the run directory, producing finished portraits with no manual handoffs.

- **Recurring scheduled work**: A team configures a cron message that runs
  weekday mornings. The daemon picks it up, routes it through a code-review
  routine, and deposits results in the run directory. The team checks
  `decree log` when they arrive.

- **Interactive prompt sessions**: A developer runs `decree prompt migration`
  to plan the next batch of work. The prompt template injects current project
  state — processed migrations, available routines, config — into the
  clipboard or launches the AI interactively with full context.

- **Retry and dead-lettering**: A routine fails due to a flaky API. Decree
  retries up to the configured limit, passing prior attempt logs so the AI
  can adjust its approach. If all retries exhaust, the message is
  dead-lettered for manual review rather than silently dropped.

## Scope

**In scope:**

- Project initialization with AI tool selection and scaffolding
- Migration-driven processing pipeline with alphabetical ordering
- Message system with YAML frontmatter, chain/sequence tracking, and routing
- Shell-based routine system with pre-checks, custom parameters, and chaining
- Lifecycle hooks (beforeAll, afterAll, beforeEach, afterEach)
- Retry with configurable max attempts and dead-letter queue
- Cron scheduling and daemon mode for continuous operation
- Run directory logging and execution audit trail
- Prompt templates with variable substitution and project context injection
- CLI for process, prompt, routine, verify, daemon, status, log, init, help
- Git stash hooks for change isolation per routine execution

**Out of scope (future work):**

- Built-in AI providers (decree invokes external tools, not APIs directly)
- Web UI or dashboard for monitoring
- Multi-project orchestration or cross-repo workflows
- Parallel migration processing (sequential by design)
- Built-in version control beyond optional git stash hooks
- User authentication or team access control

## Deliverables

1. CLI binary (`decree`) with subcommands for the full workflow lifecycle
2. Project scaffolding via `decree init` with AI tool selection
3. Migration processing pipeline with inbox/outbox message passing
4. Routine system with shell scripts, pre-checks, parameter discovery, and
   chaining
5. Lifecycle hook system for cross-cutting concerns (git stash, notifications)
6. Daemon with cron-based scheduling and inbox polling
7. Prompt template system with context-aware variable substitution
8. Execution logging with run directories, attempt tracking, and dead-letter
   queue
9. Example projects demonstrating different workflow patterns

## Acceptance Criteria

- Running `decree init` in an empty directory produces a working project
  structure with config, routines, prompts, and router
- Migration files in `.decree/migrations/` are processed in alphabetical
  order, each exactly once, tracked in `processed.md`
- A routine can chain follow-up messages that are processed depth-first
  before the next migration
- Failed routines retry up to `max_retries` times with prior logs available,
  then dead-letter
- `decree process --dry-run` lists pending work without executing anything
- `decree verify` runs all routine pre-checks and reports readiness
- `decree daemon` continuously monitors cron schedules and inbox for new
  messages
- `decree prompt` assembles templates with project context and offers
  clipboard copy or interactive AI launch
- `decree status` shows processing progress; `decree log` shows execution
  output
- Lifecycle hooks fire at the correct points with the documented environment
  variables
- SIGINT during processing exits cleanly (code 130) without running further
  retries or hooks
- The tool is AI-agnostic — routines invoke whichever AI tool the user
  configures

## Assumptions & Constraints

- Users have a Unix-like shell environment (bash) for routine execution
- At least one AI CLI tool (claude, copilot, opencode, etc.) is installed
  and accessible on PATH
- Migration files are markdown with optional YAML frontmatter
- Processing is sequential and single-threaded by design — ordering guarantees
  matter more than throughput
- Decree orchestrates AI tools but does not embed or bundle any AI provider
- Git is available if git stash hooks are configured, but git is not required
  for core functionality
