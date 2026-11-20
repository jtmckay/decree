---
machine: develop
---
# A04: `decree emit` takes a whole message on stdin

## Overview

`decree emit` treats stdin as the body only. Given a whole message file (`decree emit --machine develop < .decree/migrations/01-a.md`), it nests the file's frontmatter inside the new message's body, and it needs `--machine` repeated. The user wants rerunning a migration, or any saved message, to be one plain command.

## Outcome

- When stdin starts with frontmatter, that frontmatter is the new message's, and the rest is its body. `--machine` and `--param` override what it says; `--machine` may be left out when the frontmatter names a machine. The keys decree sets (`id`, `state`, `trigger`, `migration`) are replaced, as for any message placed in `inbox/` (A02, A03).
- `decree emit < .decree/migrations/01-a.md` queues that migration's work again as a new inbox run; the migration file and `processed.md` are untouched.
- The guidance on messages has a short "Run a message again" recipe: `decree emit < <file>`, or copying the file into `inbox/`.

## Constraints

- A body without frontmatter is emitted as today.
- Frontmatter on stdin that is not valid is refused, with the same errors `decree check` gives, and nothing is queued.
- Only this migration's scope.
- Never edit `.decree/migrations/`, `.decree/runs/` or `.decree/processed.md`, not even by a search and replace across the repository: exclude them from every bulk edit.
- If the reference docs and this migration disagree in a way you cannot settle, write the question to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- No test calls a real model or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass, and `decree check` passes in every example directory.
- CHANGELOG, the reference docs, the skill (and this repository's copy, as `decree skill` writes it) and a decision in `docs/decisions.md` say what changed and why.

## Acceptance Criteria

- **Given** a processed migration `01-a.md` for machine `m`
  **When** `decree emit < .decree/migrations/01-a.md` runs, then `process`
  **Then** a new run for `m` has the migration's body once, with no frontmatter in it

- **Given** that file
  **When** `decree emit --machine n < .decree/migrations/01-a.md` runs
  **Then** the message is for `n`

- **Given** stdin with frontmatter naming no machine and no `--machine`
  **When** `decree emit` runs
  **Then** it exits non-zero naming the missing machine, and queues nothing
