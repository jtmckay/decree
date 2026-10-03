# Scripts

A script is any executable file; the shebang picks the language (bash by default). It does one
piece of work, makes no routing decision, and reports one outcome.

## Where scripts live

Machines name scripts, never paths. Script `X` used by machine `M` resolves in this order, and
the first directory holding a match wins:

1. `.decree/scripts/M/`
2. `.decree/scripts/`
3. `<shared_source>/scripts/M/`
4. `<shared_source>/scripts/`

A match is a file named `X` or `X.<ext>` (`verify.sh`), executable, and the only match in its
directory. Script names match `^[a-z][a-z0-9_]*$`. Write generic scripts (`commit`, `notify`,
`snapshot`) once in `scripts/`; put a machine's own version in `scripts/<machine>/`.
`chmod +x` new scripts; `decree check` fails otherwise.

## How a script runs

- Directly (no shell wrapper), from the project root, with stdin `/dev/null`, in its own process
  group.
- stdout and stderr go to `runs/<id>/NNNN-<state>-<script>.log` (stderr lines prefixed
  `[stderr] `). What an invoke prints is what a later `matches` check or model reads.
- On stop or timeout, decree sends SIGTERM to the group, then SIGKILL after 10 s.
- Scripts must be safe to re-run: an interrupted step runs again after `decree retry`.

## Reporting an event (invoke only)

1. Exit non-zero: `error`.
2. Exit 0, last non-empty stdout line is a JSON object with a string `event`: that event.
3. Exit 0 otherwise: `done`.

```bash
#!/usr/bin/env bash
# scripts/feature/verify.sh: a pass is deterministic; only failures go further.
set -uo pipefail
cargo test 2>&1 | tail -n 40
if [ "${PIPESTATUS[0]}" -eq 0 ]; then
  echo '{"event":"pass"}'
else
  echo '{"event":"fail"}'
fi
exit 0
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

## Environment

| Variable | Value |
| --- | --- |
| `DECREE_PROJECT_ROOT` | Directory containing `.decree/`. |
| `DECREE_MESSAGE` | `runs/<id>/message.md` (the run's copy of the message). |
| `DECREE_MESSAGE_ID` | The message `id`. |
| `DECREE_MACHINE`, `DECREE_STATE` | Machine and state (`_root` for root `onentry`/`onexit`). |
| `DECREE_PHASE` | `onentry`, `invoke` or `onexit`. |
| `DECREE_VISITS` | Times this state has been entered, including now. |
| `DECREE_RUN_DIR` | `runs/<id>/`. |
| `DECREE_ATTEMPT`, `DECREE_MAX_ATTEMPTS`, `DECREE_FINAL_ATTEMPT` | Attempt number, the limit, and `true` on the last one. |
| `DECREE_TRIGGER` | `migration`, `cron`, `emit`, `inbox` or `invoke`. |
| `DECREE_EVENTS` | Events the current state accepts, space-separated. |
| `DECREE_PARENT` | In a child run, the parent run's id. |
| `DECREE_REQUEST`, `DECREE_REPLY` | In a router run: the request JSON to read and the reply JSON to write. |
| `DECREE_WAIT_ID`, `DECREE_QUESTION`, `DECREE_CHOICES` | For a `choose: person` `ask` script: the wait id, the question, and a JSON file of options and descriptions. |
| `DECREE_RECEIVED` | The last reply this run received (`runs/<id>/received/<file>`), or empty. |
| `DECREE_DATA_<NAME>` | Each `data` value: the message's `params`, else the default. |

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
