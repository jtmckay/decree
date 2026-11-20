---
machine: rust_develop
---
# 80: One duration format

## Overview

Machines take seconds as integers (`timeout_s: 604800`), while `decree prune` takes `--older-than 30d`. Decided: one human duration format everywhere, as Kubernetes and Go durations use.

## Requirements

Read `docs/reference/machines.md` (Invoke, Keys), `docs/reference/cli.md` (`prune`, `daemon`) and `src/commands/prune.rs` first.

1. **The format:** a whole number followed by one unit, `s`, `m`, `h` or `d`: `90s`, `10m`, `12h`, `7d`. No fractions, no combinations (`1h30m`), and no bare numbers. One parser, used everywhere.
2. **Machines:** `timeout_s` becomes `timeout` in the `script` invoke and the `person` invoke (`timeout: 7d`). The old key fails V19, naming the new form (`timeout_s is not supported: write timeout: <n>s|m|h|d`). Update the parser, validation, the JSON Schema (`src/templates/schema/machine.schema.json`, as a `pattern`), the graph labels and every machine in `examples/` and `src/templates/`.
3. **CLI:**
   - `decree prune --older-than` uses the same parser and gains `s`;
   - `decree daemon --interval` takes a duration too (`--interval 2s`; the default stays 2 s);
   - a bad duration is a usage error and exits 2.
4. **Unchanged:**
   - `events.jsonl` keeps `duration_ms` and `timeout_at`;
   - scripts keep the environment variables they have, which carry no durations.
   
   Example scripts' own timeout variables, such as `COMFY_DRAIN_TIMEOUT_S`, are the scripts' business; leave them.
5. **Docs:** the format, once, in `docs/reference/machines.md`, linked from `cli.md`. Update the skill and `help.txt`.
6. **Tests:**
   - the parser accepts each unit and rejects `1.5h`, `1h30m`, `10`, `-1m`, `1w` and empty;
   - `timeout: 1s` stops a script;
   - a `person` `timeout` is delivered as before;
   - the V19 message for `timeout_s`;
   - `prune` and `daemon` accept and reject the same strings.

- Only this migration's scope.
- If the reference docs and the code disagree, or a case is not covered here, do not guess: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- Tests build their own `.decree/` in a temp directory and never touch this repository's `.decree/`. No test calls a real LLM or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass.

## Acceptance Criteria

- **Given** `rg -n 'timeout_s' src examples docs/reference`
  **When** it runs
  **Then** it matches only the V19 message, its test, and example scripts' own variable names

- **Given** the same bad duration string
  **When** it is given to a machine, to `prune` and to `daemon`
  **Then** all three reject it
