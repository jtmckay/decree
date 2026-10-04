---
machine: rust_develop
---
# 84: CI: test every push, and report `decree check` to GitHub code scanning

## Overview

Nothing runs the tests on a push or a pull request: `.github/workflows/` is empty. Add one GitHub Actions workflow with two jobs:

- **The gate:** formatting, lints and tests, the same commands `rust_develop`'s `gate` script runs.
- **`decree check` as SARIF:** every decree project in this repository is checked with `decree check --format sarif`, and each report is uploaded to GitHub code scanning. A machine error then shows up as an annotation on the file in the pull request (`docs/reference/cli.md`, Machine-readable output).

## Requirements

Read `docs/reference/cli.md` (Machine-readable output, including its GitHub Actions snippet), `.decree/scripts/rust_develop/gate.sh`, `tests/README.md` and `CHANGELOG.md` first.

1. **`.github/workflows/ci.yml`**, named `CI`. It runs on `push` to `main` and `v0.5`, and on every `pull_request`. Top-level `permissions: contents: read`.
   - **Job `test`** (`ubuntu-latest`):
     - `actions/checkout@v4`;
     - the stable Rust toolchain with `rustfmt` and `clippy`, via `dtolnay/rust-toolchain@stable`;
     - `Swatinem/rust-cache@v2`;
     - then exactly `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test`, each as its own step.
   - **Job `decree-check`** (`ubuntu-latest`), with `permissions: security-events: write` and `contents: read`. It:
     - builds decree with `cargo build --release`;
     - runs `decree check --format sarif` in the repository root and in every `examples/*/` that has a `.decree/`, writing one SARIF file per project. Find the projects with a shell glob, not a hand-written list, so a new example is checked without editing the workflow. A failing check must not stop the remaining projects from being checked and uploaded;
     - uploads each file with `github/codeql-action/upload-sarif@v4`, giving each project its own `category`, so their results do not replace each other;
     - fails the job at the end if any project's `decree check` exited non-zero.
2. **`docs/reference/cli.md`:** keep its snippet a minimal example, and add one sentence linking this repository's `.github/workflows/ci.yml` as a complete one.
3. **`CHANGELOG.md`:** an "Added" entry under 0.5.0.
4. **`tests/README.md`:** say that CI runs the same gate.
5. **Test** (in `tests/docs_test.rs` or a new `tests/ci_test.rs`):
   - the workflow file parses as YAML;
   - its `test` job runs exactly the three commands in `.decree/scripts/rust_develop/gate.sh`, so the two cannot drift apart;
   - the `decree-check` job uses `--format sarif`.

- Only this migration's scope.
- If the reference docs and the code disagree, or a case is not covered here, do not guess: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- No test calls a real LLM or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass.

## Acceptance Criteria

- **Given** `.github/workflows/ci.yml`
  **When** it is read
  **Then** it has the `test` and `decree-check` jobs as described, and parses as YAML

- **Given** the gate script and the workflow
  **When** the test compares them
  **Then** the workflow runs exactly the gate's three commands

- **Given** a new example project added under `examples/`
  **When** the workflow runs
  **Then** it is checked and uploaded without editing the workflow
