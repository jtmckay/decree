# Machines

A machine is a statechart in `.decree/machines/<name>.yml`: a W3C SCXML document written in
YAML. Keys are SCXML's names (`initial`, `states`, `transitions`, `onentry`, `onexit`, `invoke`,
`data`, `final`); decree implements a strict subset of SCXML, plus a few marked extensions. If
you know SCXML, you know how a machine behaves.

The first line links the machine to its graph: `# Graph: ../graph/<name>.md`.

## Keys

| Where | Key | Meaning |
| --- | --- | --- |
| Root | `name` | Equals the file stem; `^[a-z][a-z0-9_]*$`. |
| Root | `description` | Required. Shown in prompts, `decree status` and graphs. |
| Root | `data` | `name: { type: string\|int\|bool, default: ... }`. Read-only; a message's `params` override defaults. Scripts see `DECREE_DATA_<NAME>`. |
| Root | `onentry`, `onexit` | Scripts run once when the run starts, and once after a root final state is entered. |
| Root | `initial`, `states` | Required. `initial` is a direct child. |
| State | `invoke` | The state's function (below). |
| State | `transitions` | `event: target`, or `event: { target, description, type: internal }`. |
| State | `onentry`, `onexit` | Scripts run every time the state is entered or exited. They produce no event. |
| State | `initial`, `states` | Make the state compound (no `invoke`). |
| State | `final: true` | A final state: only `description`, `onentry` and `emits` allowed. |
| State | `description` | Optional; the question context for a model. |
| State | `max_attempts` | Extension. Re-run a failing script invoke in place (default: config `max_attempts`). |
| State | `timeout_s` | Extension. Time limit for a script invoke; treated as a non-zero exit. |
| State | `emits` | Extension. Machines this state's scripts may `decree emit` messages for. |

Every machine has a root-level final state named `failed`.

## Invoke: the state's function

| `invoke` | What runs | Events |
| --- | --- | --- |
| `implement` | The script `implement` (see `scripts.md`). | `done` (exit 0), `error` (non-zero), or the event it prints. |
| `{ check: <condition>, input?: <state> }` | decree evaluates the condition. No AI. | `yes` or `no`. |
| `{ choose: model, question: "...", router?: <machine>, min_confidence?: 0.8, input?: <state> }` | A router machine asks a model to pick one of the state's transitions. | An option; `unsure` below `min_confidence`; `error` if the router fails. |
| `{ choose: person, question: "...", ask: <script>, timeout_s?: <s> }` | The `ask` script tells someone; the run pauses for a reply. | An option; `error` on timeout. |
| `{ machine: <name>, params?: {...} }` | The machine runs as a child run. | The child's root final state (`failed` becomes `error`). |

A state with no `invoke` and a `done` transition passes straight through.

### Choices

For `choose`, `question` is what is being decided, and the **options are the state's
transitions** except `unsure` and `error`. Every option needs a `description`: the question and
the descriptions are exactly what the model or person sees, so write them to stand alone. Quote
questions and descriptions inside `{ ... }` (`question: "Ship this build?"`): a `, ` or `?` breaks
YAML's inline maps.

`input` names a state whose latest script output the model (or a `matches` check) reads; without
it, the most recent script's output. A model always sees the message body too. A script decides
what a model sees by what it prints, which is also how to keep secrets out of a prompt.

### Routers

A `choose: model` state never calls a model itself. decree writes `request.json` (question,
options, input, message body, the run's history) and runs a **router**: an ordinary machine
(`router:` on the invoke, else `default_router` in `config.yml`; `decree init` writes
`claude_router`). Its script reads `$DECREE_REQUEST`, asks a model, and writes
`{"event": "...", "reason": "...", "confidence": 0.86}` to `$DECREE_REPLY`. decree checks the
event is an option and applies `min_confidence`. To use another model, write another router
machine and point `router:` or `default_router` at it; recalibrate `min_confidence` when you do.

### Conditions

Typed objects: one subject, one operator.

| Condition | Meaning |
| --- | --- |
| `{ matches: '<regex>' }` | The input (a state's output) matches. |
| `{ data: <name>, matches: '<regex>' }` | A string `data` value matches. |
| `{ visits: <state>, <op>: <value> }` | Times `<state>` was entered in this run. |
| `{ data: <name>, <op>: <value> }` | A `data` value. |
| `{ confidence: <state>, <op>: <number> }` | Confidence of that `choose: model` state's latest decision, 0 to 1. |

`<op>`: `equals`, `not_equals`, `less_than`, `at_most`, `more_than`, `at_least`. `<value>` is a
literal or `{ data: <name> }`. No `and`/`or`: use two `check` states in a row.

## Events and transitions

- A script's event: `done`, `error`, or a printed name. A printed event that is reserved or
  matches no transition becomes `error`.
- Selecting a transition: the current state's `transitions`, then each ancestor's, innermost
  first. An `error` nobody handles goes to `failed`; any other unhandled event becomes `error`.
- Event names match `^[a-z][a-z0-9_]*(\.[a-z0-9_]+)*$`. Scripts and options may not use `done`,
  `error`, `unsure`, or names starting with `done.` or `error.`. `yes` and `no` are fine (decree
  reads YAML 1.2, so they stay strings).
- A **compound state** loops among its children until it enters its own final child, which
  raises `done.state.<id>`; handle it on the compound (`transitions: { done.state.work: done }`).
- `onexit` runs from the innermost state outward, `onentry` from the outermost inward, as SCXML
  does. A self-transition exits and re-enters.

## Two kinds of retry

| | `max_attempts` | A transition back (`retry`) |
| --- | --- | --- |
| Means | "That crashed; run it again." | "That worked but the result is wrong; do another round." |
| Leaves the state? | No: no `onexit`/`onentry`, no new visit | Yes: `visits` + 1 |
| Bounded by | `max_attempts` | A `check` on `visits` |

## Not supported (and the alternative)

`cond` on transitions (make the decision a `check` state), `<parallel>`, `<history>`, `<send>`
(use `decree emit`), `<raise>` (a script prints its event), `<assign>` (`data` is read-only),
targetless transitions, XML. `decree check` names the alternative when it rejects one.

## Example: everything at once

```yaml
# Graph: ../graph/feature.md
name: feature
description: Implement one feature spec with an AI agent, verify it, and commit.
data:
  max_rounds: { type: int, default: 2 }
onentry: [git_baseline]            # root onentry: once, when the run starts
onexit: [notify]                   # root onexit: once, after a root final state
initial: precheck
states:
  precheck:
    invoke: precheck
    transitions: { done: work }
  work:                            # compound: the implement/verify loop as one unit
    initial: implement
    transitions: { done.state.work: done }
    states:
      implement:
        invoke: implement
        max_attempts: 2
        onentry: [snapshot]
        onexit: [collect_logs]
        transitions: { done: verify }
      verify:                      # script: prints pass or fail
        invoke: verify
        transitions: { pass: verified, fail: rounds_left }
      rounds_left:                 # deterministic: yes or no
        invoke: { check: { visits: implement, less_than: { data: max_rounds } } }
        transitions: { yes: triage, no: review }
      triage:                      # a model picks; below 0.8 it is unsure
        invoke: { choose: model, question: "Should we implement again or split the work?", min_confidence: 0.8, input: verify }
        transitions:
          retry:  { target: implement, description: The failures look fixable; implement again. }
          split:  { target: spawn_followups, description: The scope is too large; emit smaller follow-up messages. }
          unsure: { target: review }
      review:                      # a person picks; the run pauses for the reply
        invoke: { choose: person, question: "Tests still fail. What next?", ask: ask_person, timeout_s: 172800 }
        transitions:
          approve: { target: verified, description: Good enough; commit it. }
          retry:   { target: implement, description: Try again; see my note. }
          reject:  { target: failed, description: Stop. }
      verified: { final: true }    # raises done.state.work
  spawn_followups:
    invoke: spawn
    emits: [feature]
    transitions: { done: done }
  done:   { final: true, onentry: [commit] }   # migrations: processed.md is written first
  failed: { final: true }
```

## Example: composing machines

```yaml
# Graph: ../graph/ship.md
name: ship
description: Implement a feature, then deploy it.
initial: build
states:
  build:
    invoke: { machine: feature }
    transitions: { done: release }
  release:
    invoke: { machine: deploy }
    transitions: { done: done, rejected: done }
  done:   { final: true }
  failed: { final: true }
```

## Escalation

Decide the cheapest way first and pass on only what is undecided: `no` from a `check`, `unsure`
from a `choose: model` below its `min_confidence`, then a `confidence` check to decide whether a
person's time is worth it, then `choose: person`. Each threshold is a number in the machine.

## Validation

`decree check` runs rules V1–V20 (names, targets, `failed` exists, reachability, every script
name resolves, decision states cover their events, options have descriptions, `emits` and
`router` name real machines, no invoke cycles, nothing outside the SCXML subset) and M1–M3 (every
pending migration, inbox message and cron file parses and names a machine with valid `params`).
Errors look like `machines/feature.yml: work.verify: <message>`. It also warns when
`.decree/graph/` is out of date: run `decree graph`.
