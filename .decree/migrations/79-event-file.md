---
machine: rust_develop
---
# 79: Scripts report their event in `$DECREE_EVENT_FILE`

## Overview

An invoke script names its event on the last line of stdout, as `{"event": "<name>"}`. stdout is also the script's log, so anything a script happens to print last can become its event. The default router printed a model's JSON reply last, and every bare-JSON reply failed (fixed in migration 67 with a workaround line). GitHub Actions retired `::set-output` on stdout for the same reason, in favour of the `$GITHUB_OUTPUT` file. Decided: decree does the same. stdout and stderr are only logs.

## Requirements

Read `docs/reference/scripts.md` (Events from an invoke, Environment), `docs/reference/runs.md` (`events.jsonl`), `docs/reference/machines.md` and `tests/README.md` first, then `src/runtime.rs` and `src/runtime/`.

1. **The file.** Before each invoke execution (each attempt), decree creates an empty file `runs/<id>/.event` and sets `DECREE_EVENT_FILE` to its absolute path. A script names its event by writing the event name to it, for example `echo retry > "$DECREE_EVENT_FILE"`. After the script exits, decree reads the file and deletes it. `onentry` and `onexit` scripts get `DECREE_EVENT_FILE` set to the empty string: they produce no events.
2. **The rule** that replaces "Events from an invoke":
   1. The script exits non-zero: the event is `error`. The file is not read.
   2. It exits 0 and the file holds a name (surrounding whitespace and one trailing newline are trimmed): that is the event.
   3. It exits 0 and the file is empty or missing: the event is `done`.
   4. A name that is reserved, invalid, or matches no transition of the state or its ancestors becomes `error`, with `invalid_event`, as today.
   
   stdout is never parsed.
3. **`events.jsonl`.** The `transition` event's `source` value `stdout` becomes `script` ("an event the script named"). The other values are unchanged. Bump nothing else: `v` stays 1, because the format is not released.
4. **Every script that names an event** writes it to the file instead of printing it:
   - examples (`examples/feature/.decree/scripts/feature/verify.sh` and any other);
   - templates;
   - test fixtures (`tests/fixtures/scripts/print_*.sh`, renamed to say what they do);
   - the router template: its `picked` workaround line can stay as a log line, but update the comment that explains it.
   
   Recorded runs in `examples/` keep their logs as they are (they are logs), with `source` updated in their `events.jsonl`. `tests/replay_test.rs` makes its stubs write the recorded event to the file.
5. **Docs.**
   - `docs/reference/scripts.md`: the new rule and the variable, with a one-line shell and a one-line Python example. Say why: stdout is a log, and a log can end with anything.
   - Everything else that shows `{"event": …}` on stdout: the skill, `help.txt`, the README, `docs/routers.md` and `docs/services.md`.
   - `docs/decisions.md`: an entry citing GitHub Actions' move from `::set-output` to `$GITHUB_OUTPUT`.
6. **Tests:**
   - the event comes from the file;
   - a script whose stdout ends with `{"event": "x"}` but writes nothing to the file produces `done`;
   - whitespace is trimmed;
   - an invalid name becomes `error` with `invalid_event`;
   - `onentry` scripts see an empty `DECREE_EVENT_FILE`;
   - each attempt gets a fresh empty file.

- Only this migration's scope; migrations 80–83 follow.
- If the reference docs and the code disagree, or a case is not covered here, do not guess: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- Tests build their own `.decree/` in a temp directory and never touch this repository's `.decree/`. No test calls a real LLM or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass.

## Acceptance Criteria

- **Given** a script that writes `pass` to `$DECREE_EVENT_FILE` and exits 0
  **When** its state runs
  **Then** the event is `pass`, with `source: "script"`

- **Given** a script whose last stdout line is `{"event": "pass"}` and that writes nothing to the file
  **When** its state runs
  **Then** the event is `done`

- **Given** `rg -n '"event"' examples/*/.decree/scripts src/templates`
  **When** it runs
  **Then** no script prints an event on stdout (list each remaining hit and why it is not one)
