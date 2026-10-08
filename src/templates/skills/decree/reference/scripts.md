# Scripts

A script is any executable file; the shebang picks the language (bash by default). It does one
piece of work, makes no routing decision, and reports one outcome.

## Where scripts live

Machines name scripts, never paths. Script `X` used by machine `M` resolves in this order, and
the first directory holding a match wins:

1. `.decree/scripts/M/`
2. `.decree/scripts/`

There is no other search path. To share a script across projects, symlink it into `scripts/`.

A match is a file named `X` or `X.<ext>` (`verify.sh`), executable, and the only match in its
directory. Script names match `^[a-z][a-z0-9_]*$`. Write generic scripts (`commit`, `notify`,
`snapshot`) once in `scripts/`; put a machine's own version in `scripts/<machine>/`.
`chmod +x` new scripts; `decree check` fails otherwise.

Code that scripts share but decree never runs (bash functions to source, config, data such as
workflow files) goes in `.decree/lib/`, found through `$DECREE_LIB`: `. "$DECREE_LIB/ai.sh"`.
decree never resolves a script from `lib/`.

## How a script runs

- Directly (no shell wrapper), from the project root, with stdin `/dev/null`, in its own process
  group.
- stdout and stderr go to `runs/<id>/NNNN-<state>-<script>.log` (stderr lines prefixed
  `[stderr] `). What an invoke prints is what a later `matches` check or model reads. Output
  is only a log: decree never reads an event from it.
- On stop or timeout, decree sends SIGTERM to the group, then SIGKILL after 10 s.
- Scripts must be safe to re-run: an interrupted step runs again after `decree process --retry`.

## Reporting an event (invoke only)

Write the event's name to `$DECREE_EVENT_FILE`, a fresh empty file for each attempt
(`echo pass > "$DECREE_EVENT_FILE"`; in Python,
`open(os.environ["DECREE_EVENT_FILE"], "w").write("pass")`).

1. Exit non-zero: `error`. The file is not read.
2. Exit 0 and the file holds a name (whitespace trimmed): that event.
3. Exit 0 and the file is empty: `done`.
4. A name that is reserved or matches no transition of the state or its ancestors: `error`.

```bash
#!/usr/bin/env bash
# scripts/feature/verify.sh: a pass is deterministic; only failures go further.
set -uo pipefail
cargo test 2>&1 | tail -n 40
if [ "${PIPESTATUS[0]}" -eq 0 ]; then
  echo pass > "$DECREE_EVENT_FILE"
else
  echo fail > "$DECREE_EVENT_FILE"
fi
```

`onentry` and `onexit` scripts never produce events:

- A failing `onentry` script skips the remaining `onentry` scripts of that state only (SCXML stops
  the failing block, `error.execution`). The other states being entered still run theirs,
  outermost first. Once entry is complete, the event is `error`, selected from the atomic state
  entered like any other event, and the invoke does not run. This holds for root `onentry` too:
  an `error` transition can handle it, and otherwise it goes to `failed`.
- A failing `onentry` script on a root-level final state moves the run to `failed` instead of
  ending in that state, so a failed `commit` is never reported as `done`.
- A failing `onexit` script is recorded in the `transition` event under `exit_failures`; the event
  and target do not change.

`$DECREE_EVENTS` lists the events the current state accepts.

## Queueing follow-up work

```bash
#!/usr/bin/env bash
# scripts/feature/spawn.sh: the state lists `emits: [feature]`.
set -euo pipefail
decree emit --machine feature <<'EOF'
# Follow-up: split rate limiting per route
Given ... When ... Then ...
EOF
```

## Run-directory files

Scripts hand work to one another, and to a person, through files in `$DECREE_RUN_DIR`. decree
reads none of them; the built-in `develop` machine's scripts follow this convention:

- `progress.md`: the step log, one line per step, what is done and what is next. A retry
  reads it and continues.
- `STOP`: a question instead of a guess. A script that finds it prints it and writes `stop` to
  `$DECREE_EVENT_FILE`; it keeps stopping retries until a person answers and deletes it.
- `gate.log`: the output of the project's checks (`scripts/develop/gate.sh`), for the step that
  fixes them. The gate names `pass` or `fail`; a non-zero exit (`error`) means the checks could
  not run, such as a missing tool, and its message is here.
- `plan.md`: an optional plan a step writes for later ones.

## Environment

| Variable | Value |
| --- | --- |
| `DECREE_PROJECT_ROOT` | Directory containing `.decree/`. |
| `DECREE_LIB` | `.decree/lib/`: shared code, config and data. |
| `DECREE_MESSAGE` | `runs/<id>/message.md` (the run's copy of the message). |
| `DECREE_MESSAGE_ID` | The message `id`. |
| `DECREE_MACHINE`, `DECREE_STATE` | Machine and state (`_root` for root `onentry`/`onexit`). |
| `DECREE_PHASE` | `onentry`, `invoke` or `onexit`. |
| `DECREE_VISITS` | Times this state has been entered, including now. |
| `DECREE_RUN_DIR` | `runs/<id>/`. |
| `DECREE_STORE` | `.decree/store/<machine>/`: what this machine keeps between runs; created before the script runs. |
| `DECREE_ATTEMPT`, `DECREE_MAX_ATTEMPTS`, `DECREE_FINAL_ATTEMPT` | Attempt number, the limit, and `true` on the last one. |
| `DECREE_ATTEMPT_VALUE`, `DECREE_ATTEMPT_VALUES` | With an `attempts` list: this attempt's entry, and the whole list, space-separated. Unset with `attempts: <n>`; no value for `onentry`/`onexit`. |
| `DECREE_TRIGGER` | `migration`, `cron`, `emit`, `inbox` or `invoke`. |
| `DECREE_EVENTS` | Events the current state accepts, space-separated. |
| `DECREE_EVENT_FILE` | For an invoke: the file to write its event to. Empty for other scripts. |
| `DECREE_PARENT` | In a child run, the parent run's id. |
| `DECREE_PARENT_RUN_DIR` | In a child run, the parent run's folder; empty otherwise. Reach the task's files from here, never by building `runs/$DECREE_PARENT`. |
| `DECREE_ROOT_RUN_DIR` | The folder of the run at the top of the chain (no parent); `DECREE_RUN_DIR` in a top-level run. |
| `DECREE_REQUEST`, `DECREE_REPLY` | In a router run: the request JSON to read and the reply JSON to write. |
| `DECREE_WAIT_ID`, `DECREE_QUESTION`, `DECREE_CHOICES` | For a `person` `ask` script: the wait id, the question, and a JSON file of options and descriptions. |
| `DECREE_RECEIVED` | The last reply this run received (`runs/<id>/received/<file>`), or empty. |
| `DECREE_DATA_<NAME>` | Each `data` value: the message's `params`, else the default. |
| `TRACEPARENT`, `TRACESTATE` | W3C Trace Context: the run's trace and this script's span (`00-<trace id>-<span id>-01`), and the message's `tracestate` if it had one. OpenTelemetry SDKs read them, so the script's own spans join the run's trace; `decree emit` copies them into the new message. |

Project variables (a service URL, a model name) go in `.decree/env`, a committed dotenv file
every script gets: `KEY=value` per line, `#` comments, optional `export ` and quotes. Values
are interpolated as Compose's `env_file` does: `${VAR}`, `$VAR`, `${VAR:-default}`,
`${VAR-default}`, `$$` for a `$`; single-quoted values stay literal. `VAR` comes from the
process environment, then the lines above; an unknown one is empty and `decree check` warns.
Write a shared host once: `HOST=box` then `COMFY_URL=http://${HOST}:8188`. Invoke `env`
values are not interpolated. Never source it by hand. No secrets in it: they go in the process environment. One
script run by several states with different values gets them from the invoke's `env`:
`script: { name: build, env: { METHOD: image_text } }`. When two set a name, the first wins:
decree's own `DECREE_*`, the invoke's `env`, the process environment, `.decree/env`.
`DECREE_*`, `TRACEPARENT` and `TRACESTATE` cannot be set in either.

## An ask script

```bash
#!/usr/bin/env bash
# scripts/ask_person.sh: tell someone how to answer. A real one posts to chat or opens an issue.
set -euo pipefail
echo "Question: $DECREE_QUESTION"
cat "$DECREE_CHOICES"
for event in $DECREE_EVENTS; do
  [ "$event" = error ] && continue
  echo "  decree event $DECREE_WAIT_ID $event -m \"<note>\""
done
```

## Long-running services

Model servers and similar services are not scripts; run them under systemd or similar. A state
that needs one starts it in an `onentry` script, so the dependency shows in the machine and its
graph.
