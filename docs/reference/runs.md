# Runs

The interpreter is deterministic everywhere except `choose: model` and `choose: person` states, and even there the chooser can only pick one of the options the state declares, and decree validates the pick.

## Step loop

The run's current state is always an atomic state, `S`.

1. **Start or continue.** New run: append the claim event (`type: "transition"`, `from: null`, `event: "claimed"`, `to`: root `initial` followed down to an atomic state), mirror `state`, run root `onentry`, then the `onentry` of each state from root `initial` down to `S`. A `pending` run after `decree retry`: run root `onentry`, then the `onentry` of every ancestor of `S` and of `S` itself, outermost first; scripts must be safe to re-run. A `pending` run after a `received` event (a reply or a timeout): go to step 4 with that event; nothing is re-run, because the run only paused.
2. **Invoke.** Run `S`'s function ([Invoke](machines.md#invoke-the-states-function)): a script ([scripts.md](scripts.md)), a check, a choice, or a child machine (below). A `machine` or `choose: model` invoke appends `waiting` for its child run and steps the child; when the child finishes, the parent continues at step 4. A `choose: person` invoke appends `waiting`, releases the lock and ends this step; a reply or timeout continues at step 4 ([Replies](messages.md#replies)).
3. **The event.** An `onentry` failure gives `error`. Otherwise the event is what the function produced. A state with no `invoke` produces `done`.
4. **Find the target.** Select the transition for the event ([Rules](machines.md#rules)): the state's own, else the nearest ancestor's. An `error` that matches nothing targets `failed`. If the target is compound, follow `initial` down to an atomic or final state. That state is `T`.
5. **Exit.** Run `onexit` scripts from `S` outward, up to but not including the transition domain ([Rules](machines.md#rules)).
6. **Record.** Append a `transition` event, then mirror `state: T` into `message.md`.
7. **Enter.** For a migration entering a root-level final state other than `failed`, write the ledger line first ([Migrations](messages.md#migrations-ordered-run-once-stop-on-error), rule 5). Run `onentry` scripts from the first state below the transition domain down to `T`.
8. **Finish or loop.** If `T` is a final state inside a compound state `P`, raise `done.state.<P>` and go to step 4 with it, from `T`. If `T` is a root-level final state, run root `onexit`, append `run_finished`, delete the lock and end the run. Otherwise set `S = T` and go to step 2.

**Final-state `onentry` failure.** If an `onentry` script of a final state other than `failed` exits non-zero, append a `transition` event with event `error` and `to: failed`, mirror `state: failed`, remove the ledger line if one was written, run `failed`'s `onentry`, then root `onexit`. A failing `onentry` script on `failed` itself is only logged.

**Visits.** `visits.<state>` counts the `transition` events whose `to` is that state, excluding `source: "attempt"`. The claim event counts, and so does a `retry` event (it re-enters the state). Only atomic states have visits.

## Check

`{ check: <condition> }` evaluates the condition ([Invoke](machines.md#invoke-the-states-function), Conditions) against the input, `data` and visits, appends a `decision` event with the result, and produces `yes` or `no`. No script runs and nothing leaves the machine.

## Choose: model

A `choose: model` invoke asks a **router**: an ordinary machine that answers the question. decree builds the question, runs the router as a child run, and validates the answer; everything about prompts, models, retries and budgets lives in the router, so it can be read, changed and replaced like any machine.

1. **Request.** decree writes `request.json` in the child run's folder:

   ```json
   {
     "v": 1,
     "machine": "feature", "machine_description": "Implement one feature spec…",
     "state": "triage", "state_description": "",
     "question": "Should we implement again or split the work?",
     "options": [
       {"event": "retry", "description": "The failures look fixable; implement again."},
       {"event": "split", "description": "The scope is too large; emit smaller follow-up messages."}
     ],
     "min_confidence": 0.8,
     "input": "<the input state's output>",
     "message_body": "<the parent message's body>",
     "history": ["precheck: done", "implement: done", "verify: fail", "rounds_left: yes"]
   }
   ```

   Options are the state's transitions except `unsure` and `error`, in name order. `input` and `message_body` are kept apart, so a router can pass them on as structured context (Jev's `state` accepts any JSON). `input` is the input state's latest script output, as logged (stdout, and stderr lines with their `[stderr] ` prefix): a script decides what the model sees by what it prints, which is also how to keep secrets out of a prompt. `history` has one `"<from>: <event>"` entry for each `transition` event of this run so far, in order, except the claim.
2. **Route.** decree starts the router machine (the state's `router:`, else the machine named `router`) as a child run (Sub-machines, below), with `DECREE_REQUEST` and `DECREE_REPLY` set for its scripts.
3. **Reply.** The router writes `reply.json`: `{"event": "<one option>", "reason": "…", "confidence": 0.86, "probabilities": {…}}`. Only `event` is required.
4. **Validate.** If the child run ends in `failed`, or `reply.json` is missing or its `event` is not one of the options, the event is `error` with `router_error`. With `min_confidence` set, a missing or lower `confidence` produces `unsure` instead of the pick. decree appends a `decision` event with the pick, the reason, the confidence and the child run's id. Never fuzzy-match an option.


## The default router

`decree init` writes `machines/router.yml` and its script. It is a normal machine: edit it, replace it, or write another and point a state's `router:` at it. Below is the `--ai claude` version; `--ai copilot` and `--ai opencode` differ only in the script name (`ask_copilot`, `ask_opencode`) and the CLI call ([cli.md](cli.md)).

```yaml
# Graph: ../graph/router.md
name: router
description: Ask Claude to pick one of the options in the request.
initial: ask
states:
  ask:                             # renders the prompt from $DECREE_REQUEST, runs claude -p, writes $DECREE_REPLY
    invoke: ask_claude
    max_attempts: 2                # a reply that is not one of the options fails the script; it runs once more
    transitions: { done: done }
  done:   { final: true }
  failed: { final: true }
```

`ask_claude` renders this prompt, sends it to `claude -p`, takes the last JSON object from the reply (code fences and prose around it are fine), and exits non-zero unless its `event` is one of the options:

```text
You choose the next step of a workflow. Reply with only a JSON object:
{"event": "<one option name>", "reason": "<one sentence>", "confidence": <0 to 1>}

Workflow: {machine}: {machine_description}
Current step: {state}: {state_description}
Question: {question}

Output of the previous step:
{input}

Task message:
{message_body}

Run so far:
{history}

Options:
- {event}: {description}
- ...
```

`{history}` is one `- <entry>` line per `history` entry; `{input}` and `{message_body}` are inserted as they are.

**Confidence is the router's number.** Jev derives it from its probability distribution; GLiNER2 scores each label; chat models report their own, which is the least reliable. A `min_confidence` is therefore calibrated for one router: when a state switches router, or the `router` machine changes, revisit the threshold. Every `decision` event records the router, so thresholds can be checked against outcomes in Grafana. Other routers are other machines: TypeSafe Jev (hosted), Fastino's GLiNER2.5-Decide (a 1B classifier that runs locally on CPU, Apache-2.0), OpenAI's Decisions API (in preview; its schema is not public yet), a self-hosted LLM (SGLang, vLLM, Ollama), or a machine that asks a cheap model first and a stronger one only when the first is unsure. [Router machines](../routers.md) shows them; running a model server is outside decree.

## Sub-machines

An `invoke: { machine: <name> }` state, and every `choose: model`, runs a **child run**:

- It is an ordinary run in its own folder, `runs/<child id>/`, so its events and logs are separate and it shows in `decree status` and Grafana like any other. Its `message.md` has `machine`, `parent` (the parent run's id), `depth` (the parent's + 1). If that would exceed `max_depth`, no child starts: the state's event is `error`, with `router_error: "max_depth <n> reached"` on the `decision` event for `choose: model`, and `error: "max_depth <n> reached"` on the `transition` for `machine`, `trigger: invoke`, any `params`, and the parent message's body.
- The parent appends a `waiting` event naming the child (`child: <id>`), and the same process steps the child at once. When the child reaches a root final state, decree continues the parent: for a `machine` invoke it appends a `received` event whose event is the child's final state (`failed` becomes `error`); for a router it appends the `decision` event (above). A parent left waiting on a child that has already finished (a crash in between) is `pending`, and continues the same way.
- If the child pauses (a `choose: person` inside it), the parent stays `waiting` until the child finishes. If the child is interrupted, the parent waits too; `decree retry` on the child continues both.
- Child runs are never claimed from the inbox, and never count as migrations.

## Choose: person

See [Replies](messages.md#replies). The options and their descriptions are written to a JSON file whose path the `ask` script gets as `DECREE_CHOICES`, with the wait id as `DECREE_WAIT_ID`. When the reply is delivered, decree appends a `decision` event (who replied is not known to decree; the reply file is) and takes the transition.

## events.jsonl

One JSON object per line (JSON Lines), appended with a single `write` call on a file opened with `O_APPEND`. This file is both the run's record ([Lifecycle](messages.md#lifecycle), Source of truth) and its telemetry: it is designed to be shipped to Loki as it is ([observability.md](observability.md)).

Every event carries these fields, so each line stands alone in a log pipeline:

| Field | Type | Meaning |
| --- | --- | --- |
| `v` | int | Schema version, `1`. Added fields do not bump it; renamed or removed fields do. |
| `seq` | int | 1, 2, 3, … per run, across all types. |
| `ts` | string | RFC 3339 UTC with milliseconds, when the event was written. |
| `type` | string | `transition`, `script`, `decision`, `waiting`, `received`, `run_finished` or `interrupted`. |
| `run_id` | string | The message `id`. |
| `machine` | string | Machine name. |
| `trigger` | string | The message's `trigger`. |

**`transition`**: the state changed (or an attempt was recorded). The only type that state, status and visits are derived from.

| Field | Type | When | Meaning |
| --- | --- | --- | --- |
| `from` | string or null | Always | Atomic state left. `null` on the claim and `invalid_message` events. |
| `event` | string | Always | Event taken. `claimed` on the claim event. |
| `to` | string | Always | Atomic state entered (`T`). |
| `source` | string | Always | `claim`, `exit_code`, `stdout`, `attempt`, `check`, `model`, `person`, `machine`, `timeout`, `internal`, `invalid_message` or `retry`. |
| `exit_code` | int or null | Always | The invoke's exit code. `null` if there was no invoke. |
| `invalid_event` | string | [Events from an invoke](scripts.md#events-from-an-invoke), step 4 | The undeclared event the invoke printed. |
| `exit_failures` | list of strings | An `onexit` script failed | Names of the failed scripts. |
| `file` | string | Claim event | Original inbox or migration filename. |
| `error` | string | `invalid_message`, or a `machine` invoke that could not start | Validation message, or why the child did not start. |

`source` meanings: `exit_code` is `done`/`error` from the exit code (or a pass-through `done`); `stdout` is an event the script printed; `check`, `model` and `person` come from decision invokes; `machine` is the final state of a child run from a `machine` invoke (`failed` as `error`), and also `error` when no child could start; `timeout` is a `choose: person` deadline; `internal` is a `done.state.<id>` event decree raised; `retry` is written by `decree retry`.

**`script`**: one script execution finished. Written after the script exits, before any `transition` it causes.

| Field | Type | When | Meaning |
| --- | --- | --- | --- |
| `state` | string | Always | State the script ran for, or `_root`. |
| `phase` | string | Always | `onentry`, `invoke` or `onexit`. |
| `script` | string | Always | Script name. |
| `path` | string | Always | File that ran, relative to the project root. |
| `attempt` | int | Always | `DECREE_ATTEMPT`. |
| `started_at` | string | Always | RFC 3339 UTC with milliseconds. |
| `duration_ms` | int | Always | Wall time. |
| `exit_code` | int or null | Always | `null` if killed by a signal. |
| `timed_out` | bool | Timed out | Always `true` when present. |
| `log` | string | Always | Log filename in the run folder. |

**`decision`**: a `check` or `choose` invoke produced its event. Written before the `transition` it causes.

| Field | Type | When | Meaning |
| --- | --- | --- | --- |
| `state` | string | Always | The deciding state. |
| `kind` | string | Always | `check`, `model` or `person`. |
| `event` | string | Always | The event produced. |
| `condition` | object | `check` | The condition as written. |
| `options` | list of strings | `model`, `person` | The options offered. |
| `router` | string | `model` | The router machine used. |
| `child_run` | string | `model` | The router's child run id. |
| `pick` | string | `model` | The model's pick, even when the event is `unsure`. |
| `reason` | string | `model`, if given | Reason from the reply. |
| `confidence` | number | `model`, if reported | 0 to 1. |
| `probabilities` | map of string to number | `model`, if reported | Per option. |
| `router_error` | string | `model`, event `error` | Why there is no pick: the reply was rejected (and why), the router run failed, or `max_depth <n> reached`. |
| `duration_ms` | int | `model` | Wall time of the router run. |
| `reply` | string | `person` | The reply's filename under `received/`. |

**`waiting`**: the run is paused in a `choose: person` state.

| Field | Type | Meaning |
| --- | --- | --- |
| `state` | string | The `choose: person` state. |
| `wait_id` | string | `<run id>.w<seq>`; a reply must name it (or the run id). Absent when waiting for a child. |
| `child` | string | The child run waited for, when waiting on a sub-machine or a router. |
| `options` | list of strings | The options, in name order (a `choose: person` wait). |
| `timeout_at` | string or null | RFC 3339 deadline from `timeout_s`, or `null`. |

**`received`**: an external event arrived for a waiting run.

| Field | Type | When | Meaning |
| --- | --- | --- | --- |
| `wait_id` | string | A reply or a timeout | The wait it answers. Absent when a child finished. |
| `event` | string | Always | The event delivered. |
| `file` | string | A reply | The reply's filename under `received/`. |
| `child` | string | A child finished | The child run's id; `event` is its final state (`failed` as `error`). |
| `timed_out` | bool | A timeout | Always `true` when present; `event` is `error`. |

**`interrupted`**: the run stopped before a final state.

| Field | Type | Meaning |
| --- | --- | --- |
| `state` | string | State the run was in. |
| `cause` | string | `signal` (SIGINT or SIGTERM) or `crash` (found later with a stale or missing lock). |
| `script` | string | Script that was running, if known. |
