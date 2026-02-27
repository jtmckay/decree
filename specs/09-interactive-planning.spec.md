---
routine: rust-develop
---

# 09: Interactive Planning Session

## Overview

`decree plan` launches an interactive AI session pre-loaded with project
context. The user collaborates with the AI to design the work, and the AI
writes spec files to `specs/` or drops messages in `.decree/inbox/`. Once
the user exits the session, `decree process` or `decree daemon` handles
execution. Planning is just an interactive AI conversation — nothing more.

### Plan Selection

Plans are templates in `.decree/plans/`. Each `.md` file is a plan template
(e.g., `sow.md`, `spec.md`). The plan name determines which template is
loaded into the planning prompt.

```
decree plan sow                  # use the sow.md plan template
decree plan spec                 # use the spec.md plan template
decree plan                      # no plan name → prompt for selection
```

If the user provides a plan name as the first argument, that plan template
is used directly. If no plan name is provided, decree presents an
arrow-key selector with fuzzy type-ahead filtering (per spec 01
Interactive Selection UX) listing the available plans from
`.decree/plans/`. The plan name is the filename without the `.md`
extension.

## Requirements

### How It Works

For external AIs (Claude CLI, Copilot CLI):

1. **Prompt injection**: Build a planning prompt from project context
   (templates, existing specs, user description). Run `commands.planning`
   with the prompt. This seeds the AI's conversation with project context.
2. **Interactive continuation**: Exec into `commands.planning_continue`.
   The `--continue` flag auto-resumes the most recent conversation — no
   session ID is needed. The AI process replaces decree, giving the user
   a direct terminal session that continues the seeded conversation.

For the embedded AI (`decree ai`):

1. Load the model in-process
2. Send the planning prompt, display the response
3. Enter interactive REPL mode in the same terminal

The embedded path is entirely in-process — no subprocess, no external
command, no continuation step. The user's conversation continues naturally.

The embedded planning REPL reuses the context management functions from
`decree ai` — specifically `calculate_context_usage` and
`truncate_history`. These are not duplicated; the planning module calls
them from the AI module. This ensures identical context window handling
(budget calculation, pair-wise truncation, usage display) across both
`decree ai` and `decree plan`.

From the user's perspective all paths are seamless: `decree plan` opens
an AI conversation that already knows about the project.

### Config

The config (defined in spec 01) provides the command slots:

```yaml
commands:
  # Claude CLI (default):
  planning: "claude -p {prompt}"
  planning_continue: "claude --continue"

  # GitHub Copilot:
  # planning: "copilot -p {prompt}"
  # planning_continue: "copilot --continue"

  # Embedded (in-process):
  # planning: "decree ai"
  # planning_continue: ""

  router: "decree ai"
```

Template variable: `{prompt}` — the constructed planning prompt (shell-escaped).

When `commands.planning` starts with `decree ai`, the embedded model is
used in-process and `commands.planning_continue` is ignored — the REPL
continues naturally within the same process.

### Prompt Construction

The planning prompt is built from the selected plan template and project state:

```
You are a planning assistant for a software project.

## Plan Template
{contents of .decree/plans/<selected-plan>.md}

## Existing Specs
{list of files in specs/ with their titles, or "None yet"}

## User Request
The user will describe their goals interactively.

## Instructions
1. Analyse the request and existing project state.
2. Present a numbered plan summary with proposed spec files.
3. WAIT for the user to approve or request changes -- do NOT generate
   spec files until explicitly told to proceed.
4. When approved, generate each spec file using the template format:
   - Filename: NN-descriptive-name.spec.md
   - Include YAML frontmatter with `routine:` field
   - Write each file to the specs/ directory
```

Only the selected plan template is injected — not all templates. This keeps
the prompt focused on the type of planning the user chose.

### Session Continuation

For external AIs, the continue command receives full terminal control via
`exec` (or equivalent process replacement). Decree does not wrap, proxy,
or capture the AI's I/O — the AI process replaces decree. This means:

- The AI has direct access to stdin/stdout/stderr
- The AI can use its own tools (file writing, shell commands, etc.)
- When the user exits the AI session, they return to their shell prompt

Both Claude CLI and Copilot CLI support `--continue` to auto-resume the
most recent conversation without a session ID. The prompt injection step
creates that conversation; `--continue` picks it up.

### Fallback

If `commands.planning_continue` is not configured or the continue command
fails:

- Launch the AI directly in interactive mode
- Pipe the planning prompt as the first input
- The user experience is slightly different (no seamless context
  pre-loading) but functional

### Output

The AI session produces:

- **Spec files** written to `specs/` with naming `NN-name.spec.md` and
  YAML frontmatter (`routine:` field)
- Or **inbox messages** in `.decree/inbox/` for direct processing

After exiting the planning session, the user runs `decree process` or
`decree daemon` to execute the specs.

## Acceptance Criteria

- **Given** `.decree/plans/` contains SOW and spec templates and `specs/` has existing specs
  **When** the user runs `decree plan` without a plan name
  **Then** an arrow-key selector with fuzzy filtering is shown for plan selection

- **Given** the user selects a plan (or provides one as an argument)
  **When** the planning prompt is constructed
  **Then** only the selected plan template is incorporated along with existing specs and instructions

- **Given** a planning prompt has been constructed
  **When** the planning command runs (external AI)
  **Then** the AI is invoked with the prompt, seeding the conversation

- **Given** the prompt injection step completed
  **When** the interactive continuation begins
  **Then** decree execs into the continue command — the AI process replaces decree

- **Given** the AI session is active
  **When** the user interacts with the AI
  **Then** the AI has full terminal control (not a subprocess of decree)

- **Given** the embedded AI is selected for planning
  **When** the user runs `decree plan`
  **Then** the model loads in-process, sends the prompt, and enters interactive REPL mode

- **Given** the user approves the plan
  **When** the AI generates spec files
  **Then** they are written to `specs/` with proper naming and YAML frontmatter

- **Given** the configured AI does not support `--continue`
  **When** the user runs `decree plan`
  **Then** the AI launches directly in interactive mode with the prompt pre-loaded

- **Given** the user provides a plan name
  **When** running `decree plan sow`
  **Then** the sow plan template is used and the interactive session starts
  for the user to describe their goals

- **Given** no arguments are provided
  **When** running `decree plan`
  **Then** an arrow-key selector is shown, the user selects a plan, and the
  interactive session starts

- **Given** the user provides an invalid plan name
  **When** running `decree plan nonexistent`
  **Then** an error is shown listing available plan names

- **Given** the embedded AI is selected for planning
  **When** the planning REPL handles context truncation and usage display
  **Then** it uses the same `calculate_context_usage` and `truncate_history`
  functions as `decree ai` — no separate implementation
