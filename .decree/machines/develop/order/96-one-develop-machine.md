---
machine: rust_develop
---
# 96: One `develop` machine with a gate script, and the AI helper in `lib/`

## Overview

Lessons from running the built-in machines for real:

- **`develop`'s `verify` never fails.** It asks the AI to "exit 1 if anything fails", but `claude -p` exits 0 whatever the model concludes.
- **`rust_develop` exists only for its cargo gate.** One `develop` machine with a `gate` script each project fills in does the same for every language.
- **STOP and `progress.md` work.** "Write the question to `STOP` instead of guessing", and a step log in `progress.md`, are what make "can't" and "shouldn't" visible to the machine. Only `rust_develop` has them.
- **The AI helper is copied into every script.** `{ai_function}`, the usage-limit wait included, is pasted into each script by `decree init`. It belongs once in `lib/` (migration 94).

The user chose: one `develop` machine; `rust_develop` is removed.

## Requirements

1. **`.decree/lib/ai.sh`**, written by `decree init` for the chosen `--ai`: the `ai` function and its usage-limit wait (today's `src/templates/ai/claude.sh` and `plain.sh`, unchanged in behaviour). Scripts source it: `. "$DECREE_LIB/ai.sh"`. No script contains the function any more. A comment at the top shows how to add another backend, such as a local model through `opencode run --model ollama/<model>`, chosen by a variable, for use with `attempts: [local, claude]`.
2. **One `develop` machine** (`src/templates/machines/develop.yml`), keeping what works in `rust_develop`, and nothing more:
   ```yaml
   initial: precheck
   states:
     precheck:     # fails fast if the AI CLI is missing
       invoke: precheck
       transitions: { done: implement }
     implement:    # logs each step in progress.md; writes STOP instead of guessing
       invoke:
         script: { name: implement, attempts: 3 }
       transitions: { done: gate, stop: failed }
     gate:         # the project's own checks (scripts/develop/gate.sh), kept in gate.log
       invoke: gate
       transitions: { done: verify, error: fix }
     fix:          # the AI fixes what gate.log reports
       invoke:
         script: { name: fix, attempts: 3 }
       transitions: { done: final_gate, stop: failed }
     final_gate:   # the gate again; if it fails, so does the run
       invoke: gate
       transitions: { done: verify }
     verify:       # the AI checks the acceptance criteria and names pass or fail
       invoke:
         script: { name: verify, attempts: 3 }
       transitions: { pass: done, fail: failed, stop: failed }
     done:   { final: true }
     failed: { final: true }
   ```
   - **`gate.sh`** as `decree init` writes it: runs nothing but says so (`echo "gate: no checks configured; edit .decree/scripts/develop/gate.sh"`, exit 0), with commented lines for cargo (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`), npm (`npm ci && npm run lint && npm test`) and Go (`gofmt -l . | (! grep .) && go vet ./... && go test ./...`), its output tee'd to `$DECREE_RUN_DIR/gate.log`.
   - **`verify.sh`** asks the AI to end its reply with a line `VERDICT: pass` or `VERDICT: fail`, finds the last such line in the output, and writes `pass` or `fail` to `$DECREE_EVENT_FILE`. No verdict line is an error (non-zero exit), so `attempts` asks again.
   - **`implement`, `fix` and `verify`** share the STOP convention and `progress.md` as `rust_develop`'s scripts do today, and the run-directory files are documented as a convention in `scripts.md`: `progress.md` (the step log), `STOP` (a question instead of a guess; the script names `stop`), `gate.log`, and `plan.md` (an optional plan a step may write for later ones).
3. **Remove `rust_develop`** from the templates, `decree init`, docs, tests and the skill. `decree init` writes only `develop`.
4. **This repository's own `.decree/`:** `develop` (from the new template, `--ai claude`) with `scripts/develop/gate.sh` running the cargo gate, `lib/ai.sh`, and `lib/README.md`. Remove `.decree/machines/rust_develop.yml` and `.decree/scripts/rust_develop/`. Later migrations name `machine: develop`. Run `decree graph`.
5. **Docs:** README (quick start, the built-in machines), `docs/reference/cli.md` (`init`), the skill. CHANGELOG: under Changed, one `develop` machine with a gate script replaces `develop` and `rust_develop`; under Fixed, `verify` can fail.
6. **Tests:** `decree init` writes the files above for each `--ai`, and they pass `decree check`; the default gate exits 0 and says it is unconfigured; `verify.sh` with a stubbed AI writes `pass`, `fail`, or exits non-zero with no verdict; a machine run with stubbed AI and gate goes `implement → gate → verify → done`, and with a failing gate through `fix → final_gate`.

- Only this migration's scope.
- Never edit `.decree/migrations/` or `.decree/runs/`, not even by a search and replace across the repository: exclude both from every bulk edit.
- If the reference docs and this migration disagree in a way you cannot settle, write the question to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- No test calls a real model or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass, and `decree check` passes in every example directory.

## Acceptance Criteria

- **Given** `decree init --ai claude` in an empty directory
  **When** it finishes
  **Then** `.decree/machines/` has `develop.yml` and `router.yml` only, `.decree/lib/ai.sh` exists, and no script under `.decree/scripts/` defines `ai()`

- **Given** a stubbed AI whose reply ends `VERDICT: fail`
  **When** `verify` runs
  **Then** the state's event is `fail` and the run ends in `failed`

- **Given** this repository
  **When** `decree check` runs, and `rg -n rust_develop --glob '!.decree/migrations/**' --glob '!.decree/runs/**' --glob '!CHANGELOG.md'`
  **Then** the check passes, and nothing matches
