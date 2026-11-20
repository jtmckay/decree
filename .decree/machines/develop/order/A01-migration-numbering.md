---
machine: develop
---
# A01: Migration names say how to keep their order

## Overview

Migrations run in byte order of filename, which is simple and predictable, and the user keeps it. But a project numbering `1-`, `2-` … `99-` finds that `100-` sorts before `99-`: this repository did, and from 100 on every new migration drew `decree check`'s inserted-migration warning. A beta user names migrations with a letter and a number (`B17`, `C01`), which sorts after any digits.

## Outcome

- The guidance on writing a migration (skill, its reference, `docs/reference/messages.md`) says: pad the number to the width the project will need (`001-`); a project that outgrows it continues with a letter prefix (`A01-`, then `B01-`), because letters sort after digits; and `decree check` warns when a new name sorts before one that already ran.
- `decree check`'s inserted-migration warning ends with that hint, so a person who meets it learns the fix there.
- Migration order is unchanged: byte order of filename.

## Constraints

- Only this migration's scope.
- Never edit `.decree/migrations/`, `.decree/runs/` or `.decree/processed.md`, not even by a search and replace across the repository: exclude them from every bulk edit.
- If the reference docs and this migration disagree in a way you cannot settle, write the question to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- No test calls a real model or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass, and `decree check` passes in every example directory.
- CHANGELOG, the reference docs, the skill (and this repository's copy, as `decree skill` writes it) and a decision in `docs/decisions.md` say what changed and why.

## Acceptance Criteria

- **Given** processed `99-a.md` and pending `100-b.md`
  **When** `decree check` runs
  **Then** it warns, exit 0, and the warning says to pad numbers or continue with a letter prefix

- **Given** processed `99-a.md` and pending `A01-b.md`
  **When** `decree check` runs
  **Then** there is no inserted-migration warning, and `decree process --dry-run` lists `A01-b.md` next
