# Runs

The interpreter is deterministic everywhere except `model` and `person` states, and even there the chooser can only pick one of the options the state declares, and decree validates the pick.

## Step loop

The run's current state is always an atomic state, `S`.

1. **Start or continue.** New run: append the claim event (`type: "transition"`, `from: null`, `event: "claimed"`, `to`: root `initial` followed down to an atomic state), mirror `state`, run root `onentry`, then the `onentry` of each state from root `initial` down to `S`. A `pending` run after `decree process --retry`: run root `onentry`, then the `onentry` of every ancestor of `S` and of `S` itself, outermost first; scripts must be safe to re-run. A `pending` run after a `received` event (a reply or a timeout): go to step 4 with that event; nothing is re-run, because the run only paused.
2. **Invoke.** Run `S`'s function ([Invoke](machines.md#invoke-the-states-function)): a script ([scripts.md](scripts.md)), a check, a model, a person, or a child machine (below). A `machine` or `model` invoke appends `waiting` for its child run and steps the child; when the child finishes, the parent continues at step 4. A `person` invoke appends `waiting`, releases the lock and ends this step; a reply or timeout continues at step 4 ([Replies](messages.md#replies)).
3. **The event.** An `onentry` failure gives `error`. Otherwise the event is what the function produced. A state with no `invoke` produces `done`.
4. **Find the target.** Select the transition for the event ([Rules](machines.md#rules)): the state's own, else the nearest ancestor's. An `error` that matches nothing targets `failed`. If the target is compound, follow `initial` down to an atomic or final state. That state is `T`.
5. **Exit.** Run `onexit` scripts from `S` outward, up to but not including the transition domain ([Rules](machines.md#rules)).
6. **Record.** Append a `transition` event, then mirror `state: T` into `message.md`.
7. **Enter.** For a migration entering a root-level final state other than `failed`, write the ledger line first ([Migrations](messages.md#migrations-ordered-run-once-stop-on-error), rule 5). Run `onentry` scripts from the first state below the transition domain down to `T`.
8. **Finish or loop.** If `T` is a final state inside a compound state `P`, raise `done.state.<P>` and go to step 4 with it, from `T`. If `T` is a root-level final state, run root `onexit`, append `run_finished`, delete the lock and end the run. Otherwise set `S = T` and go to step 2.

**Final-state `onentry` failure.** If an `onentry` script of a final state other than `failed` exits non-zero, append a `transition` event with event `error` and `to: failed`, mirror `state: failed`, remove the ledger line if one was written, run `failed`'s `onentry`, then root `onexit`. A failing `onentry` script on `failed` itself is only logged.

**Visits.** `visits.<state>` counts the `transition` events whose `to` is that state, excluding `source: "attempt"`. The claim event counts, and so does a `retry` event (it re-enters the state). Only atomic states have visits.

## Check

`check: <condition>` evaluates the condition ([Invoke](machines.md#invoke-the-states-function), Conditions) against the named `output` state's output, `data`, visits or a model's confidence, appends a `decision` event with the result and the condition as written, and produces `true` or `false`. No script runs and nothing leaves the machine.

## Model

A `model` invoke asks a **router**: an ordinary machine that answers the question. decree builds the question, runs the router as a child run, and validates the answer; everything about prompts, models, retries and budgets lives in the router, so it can be read, changed and replaced like any machine.

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
     "input": "<the output state's output>",
     "message_body": "<the parent message's body>",
     "history": ["precheck: done", "implement: done", "verify: fail", "rounds_left: true"],
     "reply_schema": {
       "$schema": "https://json-schema.org/draft/2020-12/schema",
       "type": "object",
       "properties": {
         "event": {"enum": ["retry", "split"]},
         "confidence": {"type": "number", "minimum": 0, "maximum": 1},
         "reason": {"type": "string"},
         "probabilities": {"type": "object", "properties": {"retry": {"type": "number"}, "split": {"type": "number"}}, "additionalProperties": false}
       },
       "required": ["event"],
       "additionalProperties": false
     }
   }
   ```

   `.decree/schema/v1/request.schema.json` states this file as a JSON Schema ([Schemas](README.md#schemas)). Options are the state's transitions except `unsure` and `error`, in name order. `input` and `message_body` are kept apart, so a router can pass them on as structured context (Jev's `state` accepts any JSON). `input` is the latest script output of the invoke's `output` state, as logged (stdout, and stderr lines with their `[stderr] ` prefix), and empty when the invoke names no `output`: a model sees only what it is given, a script decides what that is by what it prints, and that is also how to keep secrets out of a prompt. `history` has one `"<from>: <event>"` entry for each `transition` event of this run so far, in order, except the claim. `reply_schema` is a JSON Schema (draft 2020-12) for `reply.json` (step 3): `event` is one of the options, `confidence` a number from 0 to 1, `reason` a string and `probabilities` a number per option; only `event` is required, and no other key is allowed. A typed router hands it unchanged to a constrained decoder (Ollama's `format`, an OpenAI-style `response_format`), so the model cannot answer outside the options ([Router machines](../routers.md)); an untyped one may ignore it, since decree validates the reply either way.
2. **Route.** decree starts the router machine (the invoke's `router:`, else the machine named `router`) as a child run (Sub-machines, below), with `DECREE_REQUEST` and `DECREE_REPLY` set for its scripts.
3. **Reply.** The router writes `reply.json`: `{"event": "<one option>", "reason": "…", "confidence": 0.86, "probabilities": {…}}`. Only `event` is required. `.decree/schema/v1/reply.schema.json` is this general shape; the request's `reply_schema` is the exact one.
4. **Validate.** If the child run ends in `failed`, or `reply.json` is missing or its `event` is not one of the options, the event is `error` with `router_error`. With `min_confidence` set, a missing or lower `confidence` produces `unsure` instead of the pick. decree appends a `decision` event with the pick, the reason, the confidence and the child run's id. Never fuzzy-match an option.


## The default router

`decree init` writes `machines/router.yml` and its script. It is a normal machine: edit it, replace it, or write another and point a `model` invoke's `router:` at it. Below is the `--ai claude` version; `--ai copilot` and `--ai opencode` differ only in the script name (`ask_copilot`, `ask_opencode`) and the CLI call ([cli.md](cli.md)).

```yaml
# yaml-language-server: $schema=../schema/v1/machine.schema.json
# Graph: ../graph/router.md
name: router
description: Ask Claude to pick one of the options in the request.
initial: ask
states:
  ask:                             # renders the prompt from $DECREE_REQUEST, runs claude -p, writes $DECREE_REPLY
    invoke:                        # a reply that is not one of the options fails the script; it runs once more
      script: { name: ask_claude, attempts: 2 }
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

Output this decision reads (empty if none):
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

**Confidence is the router's number.** Jev derives it from its probability distribution; GLiNER2.5-Decide reports the classifier's score for the label it picks; chat models report their own, which is the least reliable. A `min_confidence` is therefore calibrated for one router: when a `model` invoke switches router, or the `router` machine changes, revisit the threshold. Every `decision` event records the router, so thresholds can be checked against outcomes in Grafana. Other routers are other machines: TypeSafe Jev (hosted), Fastino's GLiNER2.5-Decide (a 1B classifier that runs locally on CPU, Apache-2.0), OpenAI's Decisions API (in preview; its schema is not public yet), a self-hosted LLM (SGLang, vLLM, Ollama), or a machine that asks a cheap model first and a stronger one only when the first is unsure. [Router machines](../routers.md) shows them; running a model server is outside decree.

## Sub-machines

An `invoke: { machine: <name> }` state, and every `model` invoke, runs a **child run**:

- It is an ordinary run in its own folder, `runs/<child id>/`, so its events and logs are separate and it shows in `decree status` and Grafana like any other. Its `message.md` has `machine`, `parent` (the parent run's id), `depth` (the parent's + 1), and `traceparent`: the parent's trace, under the span of the router's `decision` or of the `machine` invoke's wait, so the child's run span nests under it ([Traces](observability.md#traces)). If that would exceed `max_depth`, no child starts: the state's event is `error`, with `router_error: "max_depth <n> reached"` on the `decision` event for `model`, and `error: "max_depth <n> reached"` on the `transition` for `machine`, `trigger: invoke`, any `params`, and the parent message's body.
- The parent appends a `waiting` event naming the child (`child: <id>`), and the same process steps the child at once. When the child reaches a root final state, decree continues the parent: for a `machine` invoke it appends a `received` event whose event is the child's final state (`failed` becomes `error`); for a router it appends the `decision` event (above). A parent left waiting on a child that has already finished (a crash in between) is `pending`, and continues the same way.
- If the child pauses (a `person` inside it), the parent stays `waiting` until the child finishes. If the child is interrupted, the parent waits too; `decree process --retry` on the child continues both.
- Child runs are never claimed from the inbox, and never count as migrations.

## Person

See [Replies](messages.md#replies). The options and their descriptions are written to a JSON file whose path the `ask` script gets as `DECREE_CHOICES`, with the wait id as `DECREE_WAIT_ID`. When the reply is delivered, decree appends a `decision` event (who replied is not known to decree; the reply file is) and takes the transition.

## events.jsonl

One JSON object per line (JSON Lines), appended with a single `write` call on a file opened with `O_APPEND`. This file is both the run's record ([Lifecycle](messages.md#lifecycle), Source of truth) and its telemetry: it is designed to be shipped to Loki as it is ([observability.md](observability.md)). `.decree/schema/v1/events.schema.json` states every type below as a JSON Schema for one line ([Schemas](README.md#schemas)).

Four lines of the recorded `feature` migration in [`examples/feature/`](../../examples/feature/.decree/runs/01-rate-limit-upload/events.jsonl): the claim, the `verify` script, the transition its event caused, and `run_finished`:

```json
{"v":1,"seq":1,"ts":"2026-10-01T14:30:05.000Z","type":"transition","run_id":"01-rate-limit-upload","machine":"feature","trigger":"migration","trace_id":"94b30376f6a9be8a642b186df56c40ec","from":null,"event":"claimed","to":"precheck","source":"claim","exit_code":null,"file":"01-rate-limit-upload.md","span_id":"bbd03ab4bd8e2e6c"}
{"v":1,"seq":9,"ts":"2026-10-01T14:43:27.850Z","type":"script","run_id":"01-rate-limit-upload","machine":"feature","trigger":"migration","trace_id":"94b30376f6a9be8a642b186df56c40ec","state":"verify","phase":"invoke","script":"verify","path":".decree/scripts/feature/verify.sh","attempt":1,"started_at":"2026-10-01T14:41:51.650Z","duration_ms":96200,"exit_code":0,"log":"0006-verify-verify.log","span_id":"dd285b657d448ef9"}
{"v":1,"seq":10,"ts":"2026-10-01T14:43:27.855Z","type":"transition","run_id":"01-rate-limit-upload","machine":"feature","trigger":"migration","trace_id":"94b30376f6a9be8a642b186df56c40ec","from":"verify","event":"fail","to":"rounds_left","source":"script","exit_code":0}
{"v":1,"seq":27,"ts":"2026-10-01T14:51:12.340Z","type":"run_finished","run_id":"01-rate-limit-upload","machine":"feature","trigger":"migration","trace_id":"94b30376f6a9be8a642b186df56c40ec","state":"done","duration_ms":1267340}
```

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
| `trace_id` | string | The run's W3C Trace Context trace id, 32 lowercase hex: from the message's `traceparent` if valid, else random. A child run has its parent's ([Traces](observability.md#traces)). |

**`transition`**: the state changed (or an attempt was recorded). The only type that state, status and visits are derived from.

| Field | Type | When | Meaning |
| --- | --- | --- | --- |
| `from` | string or null | Always | Atomic state left. `null` on the claim and `invalid_message` events. |
| `event` | string | Always | Event taken. `claimed` on the claim event. |
| `to` | string | Always | Atomic state entered (`T`). |
| `source` | string | Always | `claim`, `exit_code`, `script`, `attempt`, `check`, `model`, `person`, `machine`, `timeout`, `internal`, `invalid_message` or `retry`. |
| `exit_code` | int or null | Always | The invoke's exit code. `null` if there was no invoke. |
| `attempt_value` | string | `source: "attempt"`, when the state's `attempts` is a list | The value of the attempt about to run: its `DECREE_ATTEMPT_VALUE`. |
| `invalid_event` | string | [Events from an invoke](scripts.md#events-from-an-invoke), step 4 | The undeclared event the invoke named. |
| `exit_failures` | list of strings | An `onexit` script failed | Names of the failed scripts. |
| `file` | string | A claim from the inbox or migrations | Original inbox or migration filename. Absent for a child run, which has none. |
| `error` | string | `invalid_message`, or a `machine` invoke that could not start | Validation message, or why the child did not start. |
| `span_id` | string | The claim (`claim` or `invalid_message`), and `retry` | The run span this event starts, 16 lowercase hex. `decree process --retry` starts a new run span, linked to the previous one. |
| `parent_span_id` | string | The claim, when the message's `traceparent` is valid | The run span's parent: the span `traceparent` names. |

`source` meanings: `exit_code` is `done`/`error` from the exit code (or a pass-through `done`); `script` is an event the script named in `$DECREE_EVENT_FILE`; `check`, `model` and `person` come from decision invokes; `machine` is the final state of a child run from a `machine` invoke (`failed` as `error`), and also `error` when no child could start; `timeout` is a `person` deadline; `internal` is a `done.state.<id>` event decree raised; `retry` is written by `decree process --retry`.

**`script`**: one script execution finished. Written after the script exits, before any `transition` it causes.

| Field | Type | When | Meaning |
| --- | --- | --- | --- |
| `state` | string | Always | State the script ran for, or `_root`. |
| `phase` | string | Always | `onentry`, `invoke` or `onexit`. |
| `script` | string | Always | Script name. |
| `path` | string | Always | File that ran, relative to the project root. |
| `attempt` | int | Always | `DECREE_ATTEMPT`. |
| `attempt_value` | string | An invoke whose `attempts` is a list | `DECREE_ATTEMPT_VALUE`: the entry this attempt ran with. |
| `started_at` | string | Always | RFC 3339 UTC with milliseconds. |
| `duration_ms` | int | Always | Wall time. |
| `exit_code` | int or null | Always | `null` if killed by a signal. |
| `timed_out` | bool | Timed out | Always `true` when present. |
| `log` | string | Always | Log filename in the run folder. |
| `span_id` | string | Always | The script's span, which its `TRACEPARENT` named. |

**`decision`**: a `check`, `model` or `person` invoke produced its event. Written before the `transition` it causes.

| Field | Type | When | Meaning |
| --- | --- | --- | --- |
| `state` | string | Always | The deciding state. |
| `kind` | string | Always | `check`, `model` or `person`. |
| `event` | string | Always | The event produced. |
| `condition` | object | `check` | The condition as written. |
| `options` | list of strings | `model`, `person` | The options offered. |
| `router` | string | `model` | The router machine used. |
| `child_run` | string | `model`, when a router run started | The router's child run id. Absent when `max_depth` stopped it from starting. |
| `pick` | string | `model`, unless the event is `error` | The model's pick, even when the event is `unsure`. |
| `reason` | string | `model`, if given | Reason from the reply. |
| `confidence` | number | `model`, if reported | 0 to 1. |
| `probabilities` | map of string to number | `model`, if reported | Per option. |
| `router_error` | string | `model`, event `error` | Why there is no pick: the reply was rejected (and why), the router run failed, or `max_depth <n> reached`. |
| `duration_ms` | int | `model` | Wall time of the router run; `0` when none started. |
| `reply` | string | `person` | The reply's filename under `received/`. |
| `span_id` | string | Always | The decision's span. For a `model` decision with a router run, the parent of that run's run span. |

**`waiting`**: the run is paused: in a `person` state for a reply, or in a `machine` or `model` state for its child run.

| Field | Type | When | Meaning |
| --- | --- | --- | --- |
| `state` | string | Always | The waiting state. |
| `wait_id` | string | A `person` wait | `<run id>.w<seq>`; a reply must name it (or the run id). |
| `options` | list of strings | A `person` wait | The options, in name order. |
| `timeout_at` | string or null | A `person` wait | RFC 3339 deadline from the `person` invoke's `timeout`, or `null` without one. |
| `child` | string | A child wait | The child run waited for (a sub-machine or a router). |

**`received`**: an external event arrived for a waiting run.

| Field | Type | When | Meaning |
| --- | --- | --- | --- |
| `wait_id` | string | A reply or a timeout | The wait it answers. Absent when a child finished. |
| `event` | string | Always | The event delivered. |
| `file` | string | A reply | The reply's filename under `received/`. |
| `child` | string | A child finished | The child run's id; `event` is its final state (`failed` as `error`). |
| `timed_out` | bool | A timeout | Always `true` when present; `event` is `error`. |
| `span_id` | string | Always | The wait's span, from the `waiting` event to this one. For a child that finished, the parent of its run span. |

**`run_finished`**: the run reached a root-level final state and root `onexit` has run. Always the run's last event, unless `decree process --retry` continues it.

| Field | Type | Meaning |
| --- | --- | --- |
| `state` | string | The final state (`done`, `failed`, …). |
| `duration_ms` | int | From the claim event to now; `0` for a message rejected at claim. |

**`interrupted`**: the run stopped before a final state.

| Field | Type | Meaning |
| --- | --- | --- |
| `state` | string | State the run was in. |
| `cause` | string | `signal` (SIGINT or SIGTERM) or `crash` (found later with a stale or missing lock). |
| `script` | string | Script that was running, if known. |
