---
machine: develop
---
# 104: `decree skill` refreshes the skill after an upgrade

## Overview

From real use: after upgrading decree there is no way to refresh `.claude/skills/decree/`. `decree init` refuses an existing `.decree/` (and never overwrites a file), so the user copied the skill from decree's source tree by hand. `decree graph` and `decree schema` already rewrite the files decree owns; the skill should work the same way. D30 removed 0.4's `skill` command when `init` began writing the skill; this restores a command for the one thing `init` cannot do, refreshing it.

## Requirements

1. **`decree skill [--ai <claude|copilot|opencode>] [--format <text|json>]`:** writes the decree skill into the project, overwriting decree's own skill files: `.claude/skills/decree/` for `claude` and `opencode`, `.github/skills/decree/` for `copilot`, as `init` does.
   - Without `--ai`, it refreshes every skill folder that exists; if none does, it picks the backend as `init` does and writes that one.
   - It writes only the files decree ships (`SKILL.md`, `reference/*.md`), removes a file in `reference/` that decree no longer ships, and leaves any other file in the folder alone.
   - It prints each file written, with `unchanged` for files already identical; `--format json` per `.decree/schema/v1/cli/skill.schema.json` (new).
   - Exit 0, or 1 when the project has no `.decree/`.
2. **Docs:** `docs/reference/cli.md` (the command row, and a short "Upgrading decree" note: install the new version, then `decree skill` and `decree schema`), README's commands table, `help.txt`, the skill itself (its commands table), `docs/decisions.md` (a new decision superseding the part of D30 that removed the command, with this reason), and CHANGELOG under Added.
3. **Tests:** fresh write, refresh over an edited `SKILL.md` (overwritten), an extra user file kept, a stale reference file removed, `unchanged` on a second run, each `--ai`, no `.decree/` → exit 1, and the JSON output against its schema.

- Only this migration's scope.
- Never edit `.decree/migrations/`, `.decree/runs/` or `.decree/processed.md`, not even by a search and replace across the repository: exclude them from every bulk edit.
- If the reference docs and this migration disagree in a way you cannot settle, write the question to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- No test calls a real model or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass, and `decree check` passes in every example directory.

## Acceptance Criteria

- **Given** a project whose `.claude/skills/decree/SKILL.md` is from an older decree, and a user file `.claude/skills/decree/notes.md`
  **When** `decree skill` runs
  **Then** `SKILL.md` matches the installed decree's, `notes.md` is unchanged, and the output lists what it wrote

- **Given** the same project
  **When** `decree skill` runs again
  **Then** every file is reported `unchanged`
