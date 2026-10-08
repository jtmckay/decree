---
machine: rust_develop
---
# 95: `enum` on string data, and a `number` type

## Overview

At work, a machine's `need` param could only be `plan` or `fix`, and the ComfyUI `method` one of seven names, but decree could not say so: only the scripts checked, at run time. And `megapixels`, a float, had to be a string. The user chose two additions, both from JSON Schema: `enum` and `number`.

## Requirements

1. **`enum`** on a `data` entry of `type: string`: a non-empty list of distinct strings. The `default` must be one of them (V14). A message's `params` value must be one of them: checked by `decree emit`, by `decree check` for pending migrations, inbox messages and cron files, and at claim (an invalid message, as for a wrong type today). The error names the param, the value and the allowed values. `enum` on another type is an error.
2. **`type: number`**: a JSON number (int or float). An int is a valid `number`. Passed to scripts as `DECREE_DATA_<NAME>` in its shortest round-trip form (`0.5`, `2`). Conditions on data compare it numerically, as `int` does.
3. **Schemas:** `machine.schema.json` (`enum`, `number`), and the request schema if it describes data types. **Docs:** `machines.md` (data row, V14), `messages.md` (params), the skill's reference. CHANGELOG "Added".
4. **Tests:** an `enum` default not in the list; a param outside the list rejected by `emit`, `check` and at claim; `number` accepts `1.5` and `2`, rejects `"x"`; a `check` comparing a number param.

- Only this migration's scope.
- Never edit `.decree/migrations/` or `.decree/runs/`, not even by a search and replace across the repository: exclude both from every bulk edit.
- If the reference docs and this migration disagree in a way you cannot settle, write the question to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- No test calls a real model or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass, and `decree check` passes in every example directory.

## Acceptance Criteria

- **Given** `data: { need: { type: string, enum: [plan, fix], default: fix } }`
  **When** `decree emit --machine m --param need=review` runs
  **Then** it exits 1 naming `need`, `review`, and `plan, fix`, and queues nothing

- **Given** `data: { megapixels: { type: number, default: 1.0 } }` and `--param megapixels=0.5`
  **When** the script runs
  **Then** `DECREE_DATA_MEGAPIXELS` is `0.5`
