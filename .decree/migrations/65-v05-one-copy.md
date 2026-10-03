---
routine: rust-develop
---
# 65: v0.5 cleanup: one copy of everything

## Overview

The decree skill exists three times (`src/templates/skills/decree/`, `.claude/skills/decree/`, `.github/skills/decree/`), the `sow` skill twice, and `mock/` repeats files that `decree init` writes. Keep one source for each, and let tests catch drift where a copy has to exist.

## Requirements

Read `docs/reference/README.md`, `docs/reference/cli.md` (`decree init`) and `mock/README.md` first.

1. `src/templates/skills/decree/` is the only copy of the decree skill. Replace `.claude/skills/decree` and `.github/skills/decree` with relative symlinks to it. Replace `.github/skills/sow` with a relative symlink to `.claude/skills/sow`. Check that `decree init` still writes real files (not symlinks) into a new project.
2. `mock/` shows a project as `decree init` writes it, so some files must exist in both places. Add a test that every file under `mock/.decree/` with a counterpart in `src/templates/` (for example `scripts/git_baseline.sh`, `scripts/snapshot.sh`, `machines/router.yml`, `scripts/router/ask_claude.sh`) is byte-identical to it. List the pairs in the test, and fail if a listed file is missing on either side.
3. Check `Dockerfile` and `.dockerignore` against the 0.5 layout and CLI: fix stale paths, commands or 0.4 terms. If the image builds a binary, `docker build` is not available in tests; check the file by reading it and say what you changed.
4. Leave `SOW.md`, `promote.sh`, `examples/` and this repository's own `.decree/` alone.

- Only this migration's scope; migrations 66–68 cover the rest of the cleanup.
- Change no behaviour of the binary. If the reference docs and the code disagree, or the docs are ambiguous here, do not guess: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes. The routine fails the run when `STOP` exists.
- Tests build their own `.decree/` in a temp directory and never touch this repository's `.decree/`. No test calls a real LLM or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass. The routine checks this itself after you finish.

## Files to Modify

- .claude/skills/decree, .github/skills/decree, .github/skills/sow — become symlinks
- tests/ — the mock-versus-templates test
- Dockerfile, .dockerignore

## Acceptance Criteria

- **Given** the repository
  **When** `git ls-files -s .claude/skills .github/skills` runs
  **Then** `decree` (both) and `.github/skills/sow` are symlinks (mode 120000) and no skill file is tracked twice

- **Given** a fresh temp directory
  **When** `decree init` runs
  **Then** its skill files are regular files with the same content as `src/templates/skills/decree/`

- **Given** a mock file that differs from its template by one byte
  **When** `cargo test` runs
  **Then** the mock-versus-templates test fails naming that pair
