---
machine: rust_develop
---
# 94: `lib/`, `.decree/env`, invoke `env:`, and the parent's run directory

## Overview

From real machines at work, four workarounds decree should make unnecessary:

1. **Shared code had nowhere to go.** Scripts are executables found in `scripts/`, so shared bash functions, config and data files (an AI helper, ComfyUI workflow files) were put in a home-made `.decree/lib/` that every script found by hard-coding `${DECREE_PROJECT_ROOT}/.decree/lib`.
2. **Project config was sourced by hand.** Every script started with `. .decree/lib/gpu.env`, and Python scripts had to run bash just to read it.
3. **One script, several states, different values.** The ComfyUI machine had seven `build_<method>` states, each running the same script, which parsed its own state name to get the method.
4. **Child runs needed their parent's folder.** A child machine built `.decree/runs/$DECREE_PARENT` by hand to reach "the task's files".

The user chose: a `lib/` convention with `DECREE_LIB`; a dotenv file `.decree/env`; an `env:` map on a script invoke; and two run-directory variables.

## Requirements

1. **`lib/`.**
   - `decree init` creates `.decree/lib/` (with a one-line `README.md` saying what goes there: code that scripts source, config and data; decree never runs anything in it).
   - Every script gets `DECREE_LIB`, the absolute path of `.decree/lib`.
   - decree never resolves a state's script from `lib/`; `decree check` is unchanged.
   - `docs/reference/README.md` (file layout), `scripts.md` (environment and a short "Shared code" section: `. "$DECREE_LIB/ai.sh"`).
2. **`.decree/env`**, standard dotenv format as Docker Compose's `env_file` and systemd's `EnvironmentFile` read it:
   - One `KEY=value` per line; blank lines and lines starting with `#` are ignored; `export ` before a key is allowed; a value may be wrapped in single or double quotes, which are removed; no variable expansion, no multi-line values. A malformed line is a `decree check` error naming the file and line (a new rule number), and `process`/`daemon` refuse to start with the same error.
   - Keys match `^[A-Za-z_][A-Za-z0-9_]*$`. A key starting with `DECREE_` or equal to `TRACEPARENT`/`TRACESTATE` is an error: those belong to decree.
   - It is optional. When it exists, every script gets its variables. **The process environment wins:** a variable already set when decree starts keeps its value, as Docker Compose does, so a deployment can override the file.
   - It is read once when `process` or `daemon` starts (and on each `daemon` pass, so edits apply without a restart).
   - It is meant to be committed: say in the docs that secrets belong in the process environment (or a file the service manager loads), not here.
3. **`env:` on a script invoke:**
   ```yaml
   build_image_text:
     invoke:
       script: { name: build, env: { METHOD: image_text } }
   ```
   - A map of keys (same pattern and reserved names as `.decree/env`) to strings, ints or bools, passed as strings. Only on `script` invokes, as `attempts` and `timeout` are.
   - It wins over `.decree/env` and the process environment, for that invoke only.
   - Schema, V-rules, docs (`machines.md` invoke table, `scripts.md` precedence: decree's own `DECREE_*` > invoke `env` > process environment > `.decree/env`).
4. **Run directories:**
   - `DECREE_PARENT_RUN_DIR`: in a child run, the absolute path of the parent run's directory. Empty otherwise.
   - `DECREE_ROOT_RUN_DIR`: the run directory of the top of the chain (the run with no parent). In a top-level run it equals `DECREE_RUN_DIR`.
   - `docs/reference/scripts.md` and the skill's reference.
5. **Tests** for each: `DECREE_LIB` set; `.decree/env` parsed per the rules above (quotes, comments, `export`, a malformed line, a reserved key, the process environment winning); invoke `env` winning; the two run-directory variables in a parent, a child and a grandchild.
6. `CHANGELOG.md` "Added" entries.

- Only this migration's scope.
- Never edit `.decree/migrations/` or `.decree/runs/`, not even by a search and replace across the repository: exclude both from every bulk edit.
- If the reference docs and this migration disagree in a way you cannot settle, write the question to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- No test calls a real model or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass, and `decree check` passes in every example directory.

## Acceptance Criteria

- **Given** `.decree/env` with `COMFY_URL="http://box:8188"` and a script that prints `$COMFY_URL`
  **When** the script runs, and again with `COMFY_URL=http://other:8188` set in decree's environment
  **Then** it prints `http://box:8188`, then `http://other:8188`

- **Given** `script: { name: build, env: { METHOD: image_text } }`
  **When** it runs
  **Then** the script sees `METHOD=image_text`, and a different state invoking `build` without `env` does not

- **Given** `.decree/env` with a line `DECREE_X=1`, or a line `not a pair`
  **When** `decree check` runs
  **Then** it exits 1 naming the file and line

- **Given** a parent run that invokes a child machine that invokes a grandchild
  **When** the grandchild's script runs
  **Then** `DECREE_PARENT_RUN_DIR` is the child's run directory and `DECREE_ROOT_RUN_DIR` is the parent's

- **Given** `decree init`
  **When** it finishes
  **Then** `.decree/lib/README.md` exists, and every script sees `DECREE_LIB`
