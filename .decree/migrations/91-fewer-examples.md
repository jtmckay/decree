---
machine: rust_develop
---
# 91: Fewer examples

## Overview

The user wants fewer, smaller examples that use the latest API (`attempts`, migration 88) and the simplest-machine style (migration 90). They chose to delete four examples:

- **`business-eval`**: chaining with `emit` is shown by `text-to-media`'s "Commissioning from another machine" (migration 89) and `feature`;
- **`sort-documents`**: its escalation ladder of routers overlaps `route-by-complexity`;
- **`text-to-speech`** and **`whisper-transcribe`**: single-machine API wrappers, covered by `text-to-media`;
- **`decree`**: a README that says this repository builds itself; one sentence in the main README covers it.

`feature` is replaced in migration 92, not here.

## Requirements

1. `git rm -r` the five directories `examples/business-eval`, `examples/sort-documents`, `examples/text-to-speech`, `examples/whisper-transcribe` and `examples/decree`.
2. **Every reference to them** goes, outside `.decree/migrations/`, `.decree/runs/` and the CHANGELOG's earlier entries: the README, `docs/` (`docs/routers.md` quotes `sort-documents`' `gliner_router.yml` and its in-machine ladder; point at `route-by-complexity`, and keep the ladder explanation with a short YAML fragment in the doc instead of a link), `examples/*/README.md`, tests, `src/` (comments, `init.rs`, `graph.rs` tests) and `tests/README.md`.
3. **Tests** that only test those examples are deleted. A test that used one of them as a fixture for decree's behaviour, not for the example itself, keeps that coverage: move the files it needs into `tests/fixtures/` and point the test there. List each such move in your reply.
4. **The main README** says in one sentence, where it describes the project, that decree is developed with itself: every change is a migration in `.decree/migrations/`, run by `decree process`.
5. **CHANGELOG.md**, under Removed: the five examples, and why in one line.

- Only this migration's scope. No change to decree's behaviour and no change to `examples/feature`.
- Do not edit `.decree/migrations/` or `.decree/runs/`.
- If a reference cannot be removed without changing what a doc teaches, write the question to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- Print the evidence for each acceptance criterion at the end of your reply.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass.

## Acceptance Criteria

- **Given** the repository
  **When** `rg -n 'business-eval|sort-documents|sort_document|text-to-speech|whisper|examples/decree' --glob '!.decree/migrations/**' --glob '!.decree/runs/**' --glob '!CHANGELOG.md'` runs
  **Then** nothing matches

- **Given** `examples/`
  **When** it is listed
  **Then** it holds `feature`, `observability`, `route-by-complexity`, `text-to-media` and `tmux-services`, and nothing else

- **Given** the test suite
  **When** it runs
  **Then** it passes, with the coverage listed in requirement 3 kept under `tests/fixtures/`
