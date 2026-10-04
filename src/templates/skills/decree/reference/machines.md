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
| State | `invoke` | The state's function (below). `max_attempts` and `timeout_s` go inside it. |
| State | `transitions` | `event: target`, or `event: { target, description, type: internal }`. |
| State | `onentry`, `onexit` | Scripts run every time the state is entered or exited. They produce no event. |
| State | `initial`, `states` | Make the state compound (no `invoke`). |
| State | `final: true` | A final state: only `description`, `onentry` and `emits` allowed. |
| State | `description` | Optional; the question context for a model. |
| State | `emits` | Extension. Machines this state's scripts may `decree emit` messages for. |

Inside a state, write keys in this order: `description`, `invoke`, `onentry`, `onexit`,
`transitions`, `emits`, then `initial` and `states` for a compound state. Don't write defaults
(no `max_attempts: 1`).

Every machine has a root-level final state named `failed`.

## Invoke: the state's function

`invoke` is a map with exactly one key, which names the kind. For `script` and `machine`, a bare
name is short for `{ name: <name> }`, and `invoke: implement` is short for
`invoke: { script: implement }`.

| `invoke` | What runs | Events |
| --- | --- | --- |
| `script: { name: <script>, max_attempts?: <n>, timeout_s?: <s> }` | The script (see `scripts.md`), re-run in place up to `max_attempts` times (default 1), each stopped after `timeout_s`. | `done` (exit 0), `error` (non-zero), or the event it prints. |
| `check: <condition>` | decree evaluates the condition. No AI. | `true` or `false`. |
| `model: { question: ..., router?: <machine>, min_confidence?: 0.8, output?: <state> }` | A router machine asks a model to pick one of the state's transitions. | An option; `unsure` below `min_confidence`; `error` if the router fails. |
| `person: { question: ..., ask: <script>, timeout_s?: <s> }` | The `ask` script tells someone; the run pauses for a reply. | An option; `error` on timeout. |
| `machine: { name: <machine>, params?: {...} }` | The machine runs as a child run. | The child's root final state (`failed` becomes `error`). |

A state with no `invoke` and a `done` transition passes straight through. Write decision invokes
in block style:

```yaml
  rounds_left:
    invoke:
      check: { visits: implement, less_than: { data: max_rounds } }
    transitions: { true: triage, false: review }   # true and false need no quotes
```

### Choices

For `model` and `person`, `question` is what is being decided, and the **options are the state's
transitions** except `unsure` and `error`. Every option needs a `description`: the question and
the descriptions are exactly what the model or person sees, so write them to stand alone. Write
prose (`question`, `description`) in block style, one key per line.

A model sees only what it is given: the latest script output of the state named by `output`, and
the message body. Without `output`, it gets no script output at all. A script decides what a model
sees by what it prints, which is also how to keep secrets out of a prompt.

### Routers

A `model` state never calls a model itself. decree writes `request.json` (question,
options, the `output` state's output as `input`, message body, the run's history) and runs a
**router**: an ordinary machine (`router:` on the invoke, else the machine named `router`, which
`decree init` writes).
Its script reads `$DECREE_REQUEST`, asks a model, and writes
`{"event": "...", "reason": "...", "confidence": 0.86}` to `$DECREE_REPLY`. decree checks the
event is an option and applies `min_confidence`. To use another model, write another router
machine and point `router:` at it, or replace `machines/router.yml`; recalibrate `min_confidence` when you do.

### Conditions

Typed objects: exactly one subject, exactly one operator.

| Subject | Value | Operators |
| --- | --- | --- |
| `output: <state>` | That state's latest script output, as logged. The state must invoke a script. | `matches` |
| `data: <name>` | A `data` value. | `<op>`; `matches` for string data |
| `visits: <state>` | Times that atomic state was entered in this run. | `<op>` |
| `confidence: <state>` | Confidence of that `model` state's latest decision, 0 to 1. | `<op>` |

`<op>`: `equals`, `not_equals`, `less_than`, `at_most`, `more_than`, `at_least`. `<value>` is a
literal or `{ data: <name> }`. For example `{ output: read_text, matches: '(?i)invoice' }`. No
`and`/`or`: use two `check` states in a row.

## Events and transitions

- A script's event: `done`, `error`, or a printed name. A printed event that is reserved or
  matches no transition becomes `error`.
- Selecting a transition: the current state's `transitions`, then each ancestor's, innermost
  first. An `error` nobody handles goes to `failed`; any other unhandled event becomes `error`.
- Event names match `^[a-z][a-z0-9_]*(\.[a-z0-9_]+)*$`. Scripts and options may not use `done`,
  `error`, `unsure`, or names starting with `done.` or `error.`. A `check` produces `true` or
  `false`; YAML reads `true:` and `false:` as booleans, and decree reads them as those events.
  `yes` and `no` are ordinary event names (decree reads YAML 1.2, so they stay strings).
- A **compound state** loops among its children until it enters its own final child, which
  raises `done.state.<id>`; handle it on the compound (`transitions: { done.state.work: done }`).
- `onexit` runs from the innermost state outward, `onentry` from the outermost inward, as SCXML
  does. A self-transition exits and re-enters.

## Two kinds of retry

| | `max_attempts` | A transition back (`retry`) |
| --- | --- | --- |
| Means | "That crashed; run it again." | "That worked but the result is wrong; do another round." |
| Leaves the state? | No: no `onexit`/`onentry`, no new visit | Yes: `visits` + 1 |
| Bounded by | `max_attempts` in the script invoke | A `check` on `visits` |

## Not supported (and the alternative)

`cond` on transitions (make the decision a `check` state), `<parallel>`, `<history>`, `<send>`
(use `decree emit`), `<raise>` (a script prints its event), `<assign>` (`data` is read-only),
targetless transitions, XML. `decree check` names the alternative when it rejects one, and names
the new shape for an old one (`choose`, `input`, a bare `matches`, `max_attempts` or `timeout_s` on
a state, `{ machine: x, params: ... }`).

## Example: everything at once

```yaml
# Graph: ../graph/feature.md
name: feature
description: Implement one feature spec with an AI agent, verify it, and commit.
data:
  max_rounds: { type: int, default: 2 }
onentry: [git_baseline]            # root onentry: once, when the run starts
onexit: [notify]                   # root onexit: once, after a root final state is entered
initial: precheck
states:
  precheck:
    invoke: precheck
    transitions: { done: work }
  work:                            # compound: the implement/verify loop as one unit
    transitions: { done.state.work: done }   # raised when work reaches its own final state
    initial: implement
    states:
      implement:
        invoke:                    # max_attempts: re-run a crashing script in place
          script: { name: implement, max_attempts: 2 }
        onentry: [snapshot]
        onexit: [collect_logs]
        transitions: { done: verify }
      verify:                      # script: prints pass or fail
        invoke: verify
        transitions: { pass: verified, fail: rounds_left }
      rounds_left:                 # deterministic check: true or false
        invoke:
          check: { visits: implement, less_than: { data: max_rounds } }
        transitions: { true: triage, false: review }
      triage:                      # a model picks one option; below the floor it is unsure
        invoke:
          model:
            question: Should we implement again or split the work?
            min_confidence: 0.8
            output: verify
        transitions:
          retry:  { target: implement, description: The failures look fixable; implement again. }
          split:  { target: spawn_followups, description: The scope is too large; emit smaller follow-up messages. }
          unsure: review
      review:                      # a person picks one option; the run pauses for the reply
        invoke:
          person:
            question: Tests still fail. What next?
            ask: ask_person
            timeout_s: 172800
        transitions:
          approve: { target: verified, description: Good enough; commit it. }
          retry:   { target: implement, description: Try again; see my note. }
          reject:  { target: failed, description: Stop. }
      verified: { final: true }    # raises done.state.work
  spawn_followups:
    invoke: spawn
    transitions: { done: done }
    emits: [feature]               # extension: machines its scripts may emit messages for
  done:   { final: true, onentry: [commit] }   # migrations: processed.md is written first, so the commit includes it
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

Decide the cheapest way first and pass on only what is undecided: `false` from a `check`, `unsure`
from a `model` below its `min_confidence`, then a `confidence` check to decide whether a
person's time is worth it, then `person`. Each threshold is a number in the machine.

## Validation

`decree check` runs rules V1–V20 (names, targets, `failed` exists, reachability, every script
name resolves, decision states cover their events, options have descriptions, `emits` and
`router` name real machines, no invoke cycles, nothing outside the SCXML subset) and M1–M3 (every
pending migration, inbox message and cron file parses and names a machine with valid `params`).
Errors look like `machines/feature.yml: work.verify: <message>`. It also warns when
`.decree/graph/` is out of date: run `decree graph`.
