---
machine: develop
---
# 102: The gate names `pass` or `fail`; an error means the checks could not run

## Overview

From real use: the `develop` template's gate sends any non-zero exit to `fix`. So "the tests failed" and "cargo is not installed" (or the network is down) look the same, and in the second case the AI tries to fix code that is fine. The user wants "checks failed" kept apart from "checks couldn't run".

## Requirements

1. **`src/templates/scripts/develop/gate.sh`:** the checks run inside the script, their output tee'd to `$DECREE_RUN_DIR/gate.log`.
   - Before running a check, the script checks that its tools exist (`command -v`); a missing tool, or anything else that stops the checks from running, is a non-zero exit with a clear message: the state's `error`.
   - The checks ran: the script writes `pass` (all passed) or `fail` (any failed) to `$DECREE_EVENT_FILE` and exits 0.
   - Unconfigured (today's default): it says so and writes `pass`.
   - Keep the commented cargo, npm and Go lines, adapted to this shape, each with its `command -v` line.
2. **`src/templates/machines/develop.yml`:**
   ```yaml
   gate:         # the project's checks: pass or fail; error means they could not run
     invoke: gate
     transitions: { pass: verify, fail: fix }
   fix:
     ...
     transitions: { done: final_gate, stop: failed }
   final_gate:   # the gate again; fail ends the run
     invoke: gate
     transitions: { pass: verify, fail: failed }
   ```
   `error` on either gate goes to `failed` implicitly, as today for other states.
3. **Docs:** README (the gate paragraph), `docs/reference/scripts.md` (the run-directory conventions table, `gate.log`), the skill, CHANGELOG under Changed: the gate names `pass` or `fail`.
4. **Tests:** `decree init` writes the new gate and machine for each `--ai`, and they pass `decree check`; with a stubbed AI: checks failing → `fix` → `final_gate`; a missing tool → `failed` without visiting `fix`; unconfigured → `pass`.
5. **Do not change this repository's own `.decree/`** (machines or scripts): migrations run under it while this one runs. The user updates it after this migration.

- Only this migration's scope.
- Never edit `.decree/migrations/`, `.decree/runs/` or `.decree/processed.md`, not even by a search and replace across the repository: exclude them from every bulk edit.
- If the reference docs and this migration disagree in a way you cannot settle, write the question to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- No test calls a real model or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass, and `decree check` passes in every example directory.

## Acceptance Criteria

- **Given** a project from `decree init` whose gate runs a check that fails
  **When** a message is processed with a stubbed AI
  **Then** the run visits `gate` (event `fail`), `fix` and `final_gate`

- **Given** a gate whose check needs a tool that is not on `PATH`
  **When** it runs
  **Then** the state's event is `error`, the run ends in `failed`, and `fix` never runs
