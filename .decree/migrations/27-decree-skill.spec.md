---
routine: rust-develop
---

# 27: Add skill install command

## Overview

Add a new `decree skill` command that installs a Decree-provided AI integration skill into either project scope or user scope, targeting either Claude or GitHub Copilot.

The command must guide the user through an interactive selection flow, install the correct files into the correct locations for the selected target, and ensure the installed materials teach the AI assistant how to operate within the Decree migration ecosystem safely and idiomatically.

## Requirements

Implement a new CLI command named `decree skill`.

The command must support an interactive flow when required arguments are omitted:

1. Ask whether to install at `project` scope or `user` scope.
2. Ask whether the target assistant is `claude` or `copilot`.
3. Confirm the destination path and files to be written before making changes.

The command must also support non-interactive invocation via flags so it can be automated in tests and scripts, for example:

- `decree skill --scope project --target claude`
- `decree skill --scope user --target copilot`

### Command behavior

- The command installs packaged Decree AI-assistant guidance files; it does not generate ad hoc prose at runtime.
- The command must be idempotent with respect to existing identical content.
- The command must avoid silently overwriting user-modified files.
- If the destination file already exists and differs from the bundled content, the command must refuse to overwrite by default and instruct the user how to proceed.
- If a force mode is implemented, it must be explicit, e.g. `--force`.

### Claude integration

For Claude, install using the recommended skill layout so the assistant can discover and use the skill naturally:

- Project scope destination: `.claude/skills/decree/SKILL.md`
- User scope destination: `~/.claude/skills/decree/SKILL.md`

If supporting files are needed, place them under the same skill directory, for example:

- `.claude/skills/decree/examples/...`
- `.claude/skills/decree/templates/...`
- `.claude/skills/decree/reference/...`

The Claude skill content must:

- Explain Decree’s migration model, including ordering by filename, immutability, self-contained migrations, and testable BDD acceptance criteria.
- Instruct Claude to prefer creating new migrations over editing processed migrations.
- Teach Claude the required migration format, including optional frontmatter with `routine:`.
- Teach Claude how to write high-quality Given / When / Then acceptance criteria.
- Teach Claude to keep migrations day-sized, independently implementable, and automatable.
- Instruct Claude to split work into the smallest feasible logical chunks — one
  migration per concern — so that each migration fits comfortably within a
  single AI context window. Frame this the way developer tickets are scoped:
  a migration that spans five subsystems is five migrations. Smaller migrations
  reduce partial-failure risk, are easier to retry, and keep the AI’s focus
  narrow enough to produce correct, reviewable output.
- Instruct Claude to place new migrations in the correct repository location for Decree migrations.
- Include practical guidance for updating the Decree ecosystem by adding scripts, docs, and markdown in a way that remains aligned with migrations and acceptance criteria.

The installed Claude skill must be repository-portable:

- When committed in project scope, opening that repository in Claude must make the skill discoverable from the repository itself.
- When installed in user scope, the skill must be available across repositories without requiring per-repo duplication.

### GitHub Copilot integration

For Copilot, install repository or user instructions using documented Copilot conventions.

Project scope destination:

- `.github/copilot-instructions.md`

User scope destination:

- install to the supported global Copilot instructions location for the environments Decree chooses to support, or fail with a clear message if user-scope Copilot installation is not supported uniformly on the current platform/editor.

If Decree supports only project-scope Copilot initially, the command must say so clearly before writing files and exit non-zero for unsupported `--scope user --target copilot`.

The Copilot instructions must:

- Explain the Decree migration contract and file format.
- Instruct Copilot to create new immutable migrations instead of editing processed ones.
- Instruct Copilot to keep changes aligned with migrations, acceptance criteria, and repository conventions.
- Emphasize that scripts, markdown, and implementation changes should be integrated through Decree’s migration workflow rather than bypassing it.
- Instruct Copilot to split work into the smallest feasible logical chunks —
  one migration per concern — to keep each migration within a focused AI
  context window and reduce partial-failure risk.

### Bundled assets

The Decree repository must contain versioned source templates for installed skill/instruction content rather than hardcoding large embedded strings inside command logic.

At minimum, bundle:

- A Claude Decree skill template
- A Copilot Decree instructions template

The installer must copy or render these assets into destination locations.

### File and directory handling

- Create parent directories if they do not exist.
- Preserve unrelated existing files in `.claude/`, `.github/`, and sibling directories.
- Write files with deterministic content for reliable snapshot-style testing.
- Normalize trailing newlines.

### UX and output

On success, print:

- what was installed,
- where it was installed,
- which assistant target it applies to,
- whether the installation is project-scoped or user-scoped,
- and any next-step guidance, such as reopening the assistant session if required.

On refusal due to conflicts, print:

- the conflicting path,
- that the file already exists with different content,
- and the available resolution path, such as backing up, removing, or re-running with an explicit overwrite flag if supported.

## Files to Modify

- `src/cli.rs` — add `Skill` subcommand with `--scope`, `--target`, and `--force` flags
- `src/main.rs` — dispatch `Skill` command
- `src/commands/mod.rs` — register new `skill` module

## Files to Create

- `src/commands/skill.rs` — install logic, destination resolution, conflict detection, and file writes
- `src/templates/skills/claude/SKILL.md` — bundled Claude Decree skill template (embedded via `include_str!`)
- `src/templates/skills/copilot/copilot-instructions.md` — bundled Copilot instructions template (embedded via `include_str!`)

## Acceptance Criteria

- **Given** a repository without a `.claude/` directory  
  **When** the user runs `decree skill --scope project --target claude`  
  **Then** the command creates `.claude/skills/decree/SKILL.md` and reports a successful project-scoped Claude installation

- **Given** a repository without a `.github/` directory  
  **When** the user runs `decree skill --scope project --target copilot`  
  **Then** the command creates `.github/copilot-instructions.md` and reports a successful project-scoped Copilot installation

- **Given** the user runs `decree skill` with no scope or target flags  
  **When** the interactive prompts are completed with `project` and `claude`  
  **Then** the command installs the Claude skill into `.claude/skills/decree/` for the current repository

- **Given** the user runs `decree skill --scope project --target claude`  
  **When** the command runs  
  **Then** no interactive prompts are shown and the file is written directly

- **Given** the user runs `decree skill` with no scope or target flags  
  **When** the interactive prompts are completed with `project` and `copilot`  
  **Then** the command installs the Copilot instructions into `.github/copilot-instructions.md` for the current repository

- **Given** the user selects `user` scope and `claude` target  
  **When** the command completes successfully  
  **Then** it installs the Decree skill into `~/.claude/skills/decree/SKILL.md`

- **Given** the user runs `decree skill --scope user --target copilot`  
  **When** the command runs  
  **Then** the command exits non-zero and prints that user-scope Copilot installation is not supported

- **Given** the destination Claude skill file already exists with byte-for-byte identical content  
  **When** the user runs the same Claude installation command again  
  **Then** the command does not rewrite the file and reports that the installation is already up to date

- **Given** the destination Copilot instructions file already exists with byte-for-byte identical content  
  **When** the user runs the same Copilot installation command again  
  **Then** the command does not rewrite the file and reports that the installation is already up to date

- **Given** the destination file already exists with content different from the bundled Decree template  
  **When** the user runs `decree skill` without an overwrite flag  
  **Then** the command exits non-zero and reports a conflict without overwriting the existing file

- **Given** the destination file already exists with content different from the bundled Decree template  
  **When** the user runs `decree skill --force`  
  **Then** the command replaces the destination file with the bundled Decree template and reports that an overwrite occurred

- **Given** the Claude skill has been installed  
  **When** the installed `SKILL.md` is read  
  **Then** it contains sections covering migration immutability, the Given/When/Then acceptance criteria format, day-sized migrations, splitting work into the smallest feasible logical chunks per concern, and the correct location for new migration files

- **Given** the Copilot instructions have been installed  
  **When** the installed `copilot-instructions.md` is read  
  **Then** it contains guidance on the Decree migration contract, migration immutability, integrating changes through migrations rather than bypassing the workflow, and splitting work into the smallest feasible logical chunks per concern

- **Given** the installer writes bundled skill assets  
  **When** automated tests read the installed files  
  **Then** the installed content matches the checked-in bundled templates exactly

- **Given** a successful installation  
  **When** the command exits  
  **Then** stdout includes the selected target, selected scope, and the full destination path for each written file
