# Scripts

A script is any executable file. decree runs it directly, so the shebang picks the language. Bash is the default for every template `decree init` writes. decree is Unix-only.

## Long-running services

Model servers, ComfyUI and other long-running processes are not scripts and are not managed by decree. A state that needs one starts it in an `onentry` script (for example `use_comfy`: use the service, or start it, wait until it answers), so the switch is visible in the machine, its graph and its events. [Long-running services](../services.md) shows how: systemd user units with `Conflicts=` for processes that must not share a GPU, llama-swap for hot-swapping model servers behind one endpoint, and a tmux layout for watching it all. decree runs scripts directly, never inside tmux: it needs exact exit codes, separate stdout and stderr, and no terminal.

## Resolution

Scripts live in one flat directory shared by every machine, with an optional per-machine directory that overrides it. Script `X` used by machine `M` resolves by checking these directories in order, and the first directory that holds a match wins:

1. `.decree/scripts/M/`
2. `.decree/scripts/`

A match is a file named exactly `X`, or `X.<ext>` with one extension (`verify.sh`, `verify.py`). The winning directory must hold exactly one match, it must be a regular file, and `mode & 0o111` must be non-zero. Anything else fails V12. There is no other search path. Script names match `^[a-z][a-z0-9_]*$`, so a script can never be confused with a per-machine directory.

The order means a generic script (`commit`, `notify`) is written once in `scripts/`, a machine that needs its own version puts it in `scripts/<machine>/`. Every `script` event records the path that was run ([events.jsonl](runs.md#eventsjsonl)), so an override is always visible.

## Execution

The same executor runs invokes and `onentry`/`onexit` scripts.

- decree executes the file itself, with no `bash` wrapper. Working directory is the project root (the directory containing `.decree/`). stdin is `/dev/null`. The parent environment is inherited, plus the variables below. There is no sandbox: a script runs as the user who runs decree, with that user's permissions ([security policy](../../SECURITY.md)).
- Each script runs in its own process group. To stop a script (on SIGINT, SIGTERM or timeout), decree sends SIGTERM to the whole group, waits up to 10 s for every process in it to exit, then sends SIGKILL.
- stdout and stderr are both read line by line, on two threads, into one log file `runs/<id>/NNNN-<state>-<script>.log`. `NNNN` is a counter of script executions in this run, starting at `0001`: 4 digits, zero-padded, and more digits past `9999`; every attempt gets its own number. stderr lines get the prefix `[stderr] ` (with one trailing space). Root `onentry` and `onexit` scripts use `_root` as the state.
- While a script runs, `runs/<id>/.running` holds one JSON object: `{"pid": 4242, "state": "implement", "phase": "invoke", "script": "implement", "started_at": "<RFC 3339 UTC, ms>", "log": "0004-implement-implement.log"}`. decree writes it (temp file plus rename) right after spawning and deletes it after writing the `script` event. `decree status` and `decree tail` read it; it is not part of the record. A `.running` left behind by a crash names the `script` of the `interrupted` event.
- The log of a state's latest script run is that state's output: what a `{ output: <state>, matches: … }` check tests and what a `model` with `output: <state>` reads ([Invoke](machines.md#invoke-the-states-function), Output).
- **Timeout.** If the script invoke sets `timeout` ([Durations](machines.md#durations)) and the invoke runs longer, decree stops it and treats it as a non-zero exit. Its `script` event records `"timed_out": true`.
- **Attempts.** If the invoke exits non-zero (or times out) and the state has attempts left, decree runs it again without leaving the state: no `onexit` or `onentry`, one `transition` event with `event: "error"`, `from` and `to` equal, `source: "attempt"`, and the next attempt's `attempt_value` when it has one. The attempts are the script invoke's `attempts` ([Invoke](machines.md#invoke-the-states-function)): a number, or a list whose entries run in order, each as `DECREE_ATTEMPT_VALUE`; default 1. The first attempt that does not end in `error` decides the event; only when attempts run out does `error` take its transition. Each visit starts again at attempt 1. `onentry` failures are not retried.
- **Signals.** If decree receives SIGINT or SIGTERM while a script runs, it stops the script, writes no `script` event for it, and interrupts the run ([Replies](messages.md#replies), Stopping). `decree process --retry` re-runs the step, so scripts must be safe to re-run.
- **Logs** are capped at 2 MiB (2097152 bytes). After the script exits, a larger log keeps only its last 2 MiB, behind the line `[log truncated — showing last 2MB of output]`.

## Environment

A script inherits decree's environment, with every inherited `DECREE_*` variable and `TRACESTATE` removed, then gets the variables below. So a decree started inside another decree run (a test suite run by a gate script, say) never hands the outer run's variables to its own scripts.

| Variable | Value |
| --- | --- |
| `DECREE_PROJECT_ROOT` | Absolute path of the directory containing `.decree/`. |
| `DECREE_MESSAGE` | Absolute path of `runs/<id>/message.md`. |
| `DECREE_MESSAGE_ID` | The message `id`. |
| `DECREE_MACHINE` | Machine name. |
| `DECREE_STATE` | State the script runs for, or `_root`. |
| `DECREE_PHASE` | `onentry`, `invoke` or `onexit`. |
| `DECREE_VISITS` | `visits.<state>` for this state, including the current visit. `0` for `_root`. |
| `DECREE_RUN_DIR` | Absolute path of `runs/<id>/`. |
| `DECREE_ATTEMPT` | Attempt number of the invoke in this visit, from 1. `1` for `onentry` and `onexit` scripts. |
| `DECREE_ATTEMPT_VALUE` | This attempt's entry in the invoke's `attempts` list, e.g. `claude`. Unset with the integer form, and for `onentry` and `onexit` scripts. |
| `DECREE_ATTEMPT_VALUES` | The whole `attempts` list, space-separated. Unset with the integer form. |
| `DECREE_MAX_ATTEMPTS` | Attempts allowed for this state: the `attempts` list's length, or the integer. |
| `DECREE_FINAL_ATTEMPT` | `true` if `DECREE_ATTEMPT` equals `DECREE_MAX_ATTEMPTS`, else `false`. |
| `DECREE_TRIGGER` | The message's `trigger`. |
| `DECREE_EVENTS` | The events the current state accepts, space-separated, in name order. Lets a script check what it may name. |
| `DECREE_EVENT_FILE` | For a script invoke: absolute path of `runs/<id>/.event`, created empty before each attempt, where the script names its event ([Events from an invoke](#events-from-an-invoke)). Empty for `onentry` and `onexit` scripts and the `ask` script of a `person` state, which produce no events. |
| `DECREE_PARENT` | In a child run: the parent run's id. Empty otherwise. |
| `DECREE_REQUEST` | In a router run: absolute path of the request JSON ([Model](runs.md#model)). Empty otherwise. |
| `DECREE_REPLY` | In a router run: absolute path where the reply JSON must be written. Empty otherwise. |
| `DECREE_WAIT_ID` | For the `ask` script of a `person` state: the wait id a reply must name. Empty otherwise. |
| `DECREE_QUESTION` | For the `ask` script of a `person` state: its `question`. Empty otherwise. |
| `DECREE_CHOICES` | For the `ask` script: absolute path of a JSON file mapping each option to its description. Empty otherwise. |
| `DECREE_RECEIVED` | Absolute path of the last reply this run received (`runs/<id>/received/<file>`), or empty. |
| `DECREE_DATA_<NAME>` | One per `data` entry, `<NAME>` uppercased: the message's `params` value, else the default. Ints as decimal, bools as `true` or `false`. |
| `TRACEPARENT` | W3C Trace Context `00-<trace id>-<span id>-01`: the run's trace and this script execution's own span, the `span_id` of its `script` event. Set for every script, replacing any `TRACEPARENT` decree inherited ([Traces](observability.md#traces)). |
| `TRACESTATE` | The message's `tracestate`, when it carried one beside a valid `traceparent`; unset otherwise. |

`TRACEPARENT` and `TRACESTATE` follow OpenTelemetry's environment variable carrier (the `traceparent` and `tracestate` keys of W3C Trace Context, uppercased), so an OpenTelemetry SDK in the script, or a tool that reads them, makes its own spans children of the script's span, and `decree emit` puts them in the message it queues ([Traces](observability.md#traces)).

## Events from an invoke

A script names its event by writing the name to the file `$DECREE_EVENT_FILE`:

```bash
echo pass > "$DECREE_EVENT_FILE"
```

```python
open(os.environ["DECREE_EVENT_FILE"], "w").write("pass")
```

Before each attempt, decree creates `runs/<id>/.event` empty and sets `DECREE_EVENT_FILE` to its absolute path. After the script exits, decree reads the file and deletes it. Then:

1. It exits non-zero: the event is `error`. The file is not read.
2. It exits 0 and the file holds a name (surrounding whitespace, a trailing newline included, is trimmed): that is the event.
3. It exits 0 and the file is empty or missing: the event is `done`.
4. A name from step 2 that is reserved ([Rules](machines.md#rules)), invalid, or that matches no transition of the state or its ancestors: the event becomes `error`, and the `transition` event records `"invalid_event": "<name>"`.

stdout and stderr are only the script's log; decree never parses them. A log can end with anything, a model's JSON reply or a test runner's summary, so an event read from it could be one the script never meant to raise. GitHub Actions retired `::set-output` on stdout for `$GITHUB_OUTPUT` for the same reason ([decisions.md](../decisions.md#d48-scripts-name-their-event-in-decree_event_file)).

`onentry` and `onexit` scripts never produce events:

- An `onentry` script exits non-zero: skip the remaining `onentry` scripts of that state only (SCXML stops the failing block, `error.execution`). The other states being entered still run theirs, outermost first. Once entry is complete, the event is `error`, selected from the atomic state entered like any other event ([Rules](machines.md#rules), Selecting a transition), and the invoke does not run. This holds for root `onentry` too: an `error` transition can handle it, and otherwise it goes to `failed`.
- A root-level final state's `onentry` script exits non-zero: the run moves to `failed` instead of ending in that state. (SCXML would end in it; decree does not report a failed `commit` as `done`.)
- An `onexit` script exits non-zero: record it in the `transition` event under `exit_failures` and continue. The event and target do not change.

## Example scripts

`scripts/feature/verify.sh` (a `feature`-only script): a test pass is deterministic, and only failures go to the LLM.

```bash
#!/usr/bin/env bash
set -uo pipefail
cargo test 2>&1 | tail -n 40
if [ "${PIPESTATUS[0]}" -eq 0 ]; then
  echo pass > "$DECREE_EVENT_FILE"
else
  echo fail > "$DECREE_EVENT_FILE"
fi
```

`scripts/feature/spawn.sh` (also `feature`-only): follow-ups go through `decree emit`, never by writing files.

```bash
#!/usr/bin/env bash
set -euo pipefail
decree emit --machine feature <<'EOF'
# Follow-up: split rate limiting per route
Given ... When ... Then ...
EOF
```
