---
routine: develop
---
# 33: Remove the `decree prompt` Command

## Overview

Remove the `decree prompt` command. Its purpose — building context-aware
prompts and launching interactive AI — is superseded by AI-native skills.
Before deleting the prompt templates, convert the two that carry genuine
standalone value into skills. The migration authoring template (`migration.md`)
is already covered by the decree skill and needs no conversion. The routine
authoring guide (`routine.md`) is detailed enough to warrant its own skill.
The SOW template (`sow.md`) is decree-independent and becomes a standalone SOW
skill. After converting those two, remove the prompt command and all supporting
infrastructure.

## Requirements

### Part 1 — Convert prompt templates to skills

1. **Create a `decree-routine` skill** at
   `src/templates/skills/claude/decree-routine/SKILL.md`. Base it on the
   content of `src/templates/routine.md`, adapted for skill format:
   - Add YAML frontmatter:
     ```yaml
     ---
     name: decree-routine
     description: Authoring guide for Decree routine scripts
     ---
     ```
   - Remove the `{routines}` substitution placeholder (a live list is not
     available in a skill; the AI reads `.decree/routines/` directly).
   - Remove tutorial "run this command" framing; state facts declaratively.
   - Keep all technical content: required script structure, env var tables,
     custom parameter discovery algorithm, pre-check section requirements,
     nested routine layout, and tips.
   - In Claude Code this skill is invocable as `/decree-routine`.

2. **Create a `sow` skill** at `src/templates/skills/claude/sow/SKILL.md`.
   Base it on `src/templates/sow.md`, adapted for skill format:
   - Add YAML frontmatter:
     ```yaml
     ---
     name: sow
     description: Guide for writing a Statement of Work document
     ---
     ```
   - Keep all content as-is; it has no substitution placeholders and requires
     no structural changes.
   - In Claude Code this skill is invocable as `/sow`.

3. **Refactor `decree skill` to present a multi-skill selection UI.**

   The command currently resolves `--scope` and `--target` then installs a
   single file. After this migration it installs one or more skills chosen
   interactively, following the same patterns already used in `commands/skill.rs`
   and `commands/routine.rs`.

   **Skill registry** — define a static list of `SkillEntry` structs (name,
   description, bundled content, install path per scope). The Claude entries
   are:

   | Name | Description | Project path | User path |
   |---|---|---|---|
   | `decree` | Work within the Decree migration ecosystem safely and idiomatically | `.claude/skills/decree/SKILL.md` | `~/.claude/skills/decree/SKILL.md` |
   | `decree-routine` | Authoring guide for Decree routine scripts | `.claude/skills/decree-routine/SKILL.md` | `~/.claude/skills/decree-routine/SKILL.md` |
   | `sow` | Guide for writing a Statement of Work document | `.claude/skills/sow/SKILL.md` | `~/.claude/skills/sow/SKILL.md` |

   Copilot installs a single file (`.github/copilot-instructions.md`) and is
   unchanged — no multi-select applies.

   **Interactive flow for Claude target (TTY only):**

   After scope is resolved, present an `inquire::MultiSelect` prompt:

   ```
   Which skills would you like to install? (scope: project)
   > [ ] decree            Work within the Decree migration ecosystem safely and idiomatically
     [ ] decree-routine    Authoring guide for Decree routine scripts
     [ ] sow               Guide for writing a Statement of Work document
   [↑↓ move  space toggle  a toggle all  enter confirm]
   ```

   - Use `inquire::MultiSelect::new(...)` — already available in `inquire 0.7`,
     no new dependency.
   - Each option is formatted as `{name:<18} {description}` so descriptions
     align.
   - Pre-select `decree` by default (`with_default(&[0])`).
   - Show the hint line: `↑↓ move  space toggle  a toggle all  enter confirm`.
   - If the user confirms with **no skills checked** (empty selection), fall
     back to installing only the first skill in the list (`decree`) — treat the
     empty case the same way `inquire::Select` treats pressing Enter on the
     highlighted item.
   - If the user cancels (Ctrl-C / Esc), exit cleanly with no error output
     (same pattern as the existing `selection cancelled` handling in
     `commands/routine.rs`).

   **Non-TTY / `--force` flag:** when not running in a TTY, require an
   explicit `--skill <name>` flag (repeatable) to name which skills to install,
   or install all skills when `--all` is passed. Non-TTY without either flag
   should print an error and exit non-zero.

   **Per-skill install logic** — for each selected skill:
   - Compute its destination path from the scope.
   - If the file already exists and matches the bundled content, print
     `Already up to date: <path>` and skip.
   - If the file exists with different content and `--force` is not set, print
     a conflict error and skip (do not abort remaining skills).
   - Otherwise write the file, creating parent directories as needed, and print
     `Installed: <path>` (or `Overwrote: <path>` when `--force` applies).
   - After all skills are processed, print a single summary line:
     `N skill(s) installed, M already up to date, K conflict(s).`

   **Next-step hint** — after the summary, print the appropriate next-step
   message for the target (same text as today, applied once regardless of how
   many skills were installed).

   **CLI flags** — add `--skill <name>` (repeatable, for non-TTY / scripting)
   and `--all` to `src/cli.rs` on the `Skill` subcommand. Keep `--scope`,
   `--target`, and `--force` as-is.

### Part 2 — Remove the `decree prompt` command

4. **Remove the `Prompt` subcommand** from `src/cli.rs`.

5. **Delete `src/commands/prompt.rs`** — all logic for template scanning,
   variable substitution, clipboard copying, raw terminal mode, and AI
   exec-launch.

6. **Remove `pub mod prompt;`** from `src/commands/mod.rs`.

7. **Remove the `Command::Prompt` match arm** from `src/main.rs`.

8. **Remove prompt template scaffolding from `decree init`**:
   - Remove the block that writes `migration.md`, `sow.md`, and `routine.md`
     into `.decree/prompts/` in `src/commands/init.rs`.
   - Remove the `include_str!` constants `SOW_PROMPT_MD`, `MIGRATION_PROMPT_MD`,
     and `ROUTINE_PROMPT_MD`.
   - Remove the `prompts_base` variable and the three `std::fs::write` calls.

9. **Remove `PROMPTS_DIR` and `resolved_shared_prompts_dir`** from
   `src/config.rs`, including the associated test.

10. **Delete the prompt template source files** from `src/templates/`:
    - `migration.md` — content already in the decree skill; no conversion needed
    - `routine.md` — converted to `decree-routine` skill in Part 1
    - `sow.md` — converted to `sow` skill in Part 1

11. **Update `src/templates/help.txt`**:
    - Remove `decree prompt [NAME]` from the Commands section.
    - Remove `decree prompt routine` from the Getting Started and Defining
      Routines sections.
    - In the AI Skill Installation section, list all three skills (`decree`,
      `decree-routine`, `sow`) with their descriptions and install paths.

12. **Remove any integration tests** in `tests/integration_test.rs` that
    exercise `decree prompt`.

13. **Confirm `cargo build` succeeds** with zero errors and zero warnings.

## Files to Modify

- `src/templates/skills/claude/decree-routine/SKILL.md` — create
- `src/templates/skills/claude/sow/SKILL.md` — create
- `src/commands/skill.rs` — refactor for multi-skill selection UI
- `src/cli.rs` — add `--skill` / `--all` flags; remove `Prompt` variant
- `src/commands/mod.rs` — remove `pub mod prompt;`
- `src/commands/prompt.rs` — delete
- `src/main.rs` — remove `Command::Prompt` arm
- `src/commands/init.rs` — remove prompt scaffolding and `include_str!` constants
- `src/config.rs` — remove `PROMPTS_DIR`, `resolved_shared_prompts_dir()`, test
- `src/templates/migration.md` — delete
- `src/templates/routine.md` — delete
- `src/templates/sow.md` — delete
- `src/templates/help.txt` — remove prompt references, update skill installation section
- `tests/integration_test.rs` — remove prompt tests if any

## Acceptance Criteria

- **Given** the file `src/templates/skills/claude/decree-routine/SKILL.md`
  **When** its content is inspected
  **Then** it contains the required script structure, env var table, custom
  parameter discovery algorithm, pre-check requirements, and tips — and does
  not contain the string `{routines}`

- **Given** the file `src/templates/skills/claude/sow/SKILL.md`
  **When** its content is inspected
  **Then** it contains the SOW structure, writing guidelines, and example from
  the original `sow.md` template

- **Given** a TTY session running `decree skill` with no flags
  **When** the scope and target prompts are answered (project, claude)
  **Then** an `inquire::MultiSelect` is presented listing `decree`,
  `decree-routine`, and `sow` with the `decree` entry pre-checked

- **Given** the `MultiSelect` prompt is open
  **When** the user presses Space on `sow` and then Enter
  **Then** both `decree` and `sow` are installed and the summary line reports
  `2 skill(s) installed`

- **Given** the `MultiSelect` prompt is open with no items checked
  **When** the user presses Enter
  **Then** only the `decree` skill is installed (empty-selection fallback)

- **Given** `decree skill --scope project --target claude --skill decree-routine`
  is run in a non-TTY environment
  **When** the command completes
  **Then** `.claude/skills/decree-routine/SKILL.md` is written and exit code is 0

- **Given** `decree skill --scope project --target claude --all` is run
  **When** the command completes
  **Then** all three Claude skills are installed under `.claude/skills/`

- **Given** a compiled `decree` binary
  **When** `decree --help` is run
  **Then** `prompt` does not appear in the output

- **Given** a compiled `decree` binary
  **When** `decree prompt` is run
  **Then** the command exits non-zero with an "unrecognized subcommand" error

- **Given** a project directory with no `.decree/` directory
  **When** `decree init` is run
  **Then** no `.decree/prompts/` directory is created

- **Given** the source tree after the migration
  **When** `cargo build` is run
  **Then** it succeeds with zero errors and zero warnings about unused items
