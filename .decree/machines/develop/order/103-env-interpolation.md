---
machine: develop
---
# 103: `${VAR}` interpolation in `.decree/env`, as Docker Compose does

## Overview

From real use: `.decree/env` has no variable expansion, so a host's address was repeated in three URLs. Docker Compose's `env_file` interpolates; decree's file should behave the same way, so people do not have to learn a second set of rules.

## Requirements

1. **Interpolation in values**, following Docker Compose's `env_file` rules (https://docs.docker.com/compose/how-tos/environment-variables/variable-interpolation/), the subset:
   - `${VAR}` and `$VAR`: the value of `VAR`;
   - `${VAR:-default}`: `default` when `VAR` is unset or empty; `${VAR-default}`: when unset;
   - `$$`: a literal `$`;
   - single-quoted values are literal: no interpolation, as in Compose.
   - `VAR` is looked up in decree's process environment first, then in the lines of `.decree/env` above it (as their effective values, after the process environment wins). Unknown variables are empty, as in Compose, and `decree check` warns about each (`.decree/env:4: ${HOST} is not set`).
   - Invoke `env:` values are not interpolated (they are YAML, and the YAML stays literal).
2. **Errors:** an unterminated `${` or a malformed `${VAR:…}` is a `decree check` error naming the file and line, as other malformed lines are.
3. **Docs:** `docs/reference/scripts.md` (`.decree/env`), with an example:
   ```sh
   HOST=192.168.1.20
   OLLAMA_URL=http://${HOST}:11434
   COMFY_URL=http://${HOST}:8188
   ```
   The skill's reference, and CHANGELOG under Added.
4. **Tests:** each form above, precedence (process environment over the file, for both the referenced variable and the key), single quotes, `$$`, an unknown variable (empty plus a warning), and the malformed cases.

- Only this migration's scope.
- Never edit `.decree/migrations/`, `.decree/runs/` or `.decree/processed.md`, not even by a search and replace across the repository: exclude them from every bulk edit.
- If the reference docs and this migration disagree in a way you cannot settle, write the question to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- No test calls a real model or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass, and `decree check` passes in every example directory.

## Acceptance Criteria

- **Given** `.decree/env` with `HOST=box` and `URL=http://${HOST}:8188`
  **When** a script prints `$URL`, and again with `HOST=other` in decree's environment
  **Then** it prints `http://box:8188`, then `http://other:8188`

- **Given** `A='${B}'` and `C=$$5`
  **When** a script prints them
  **Then** it prints `${B}` and `$5`
