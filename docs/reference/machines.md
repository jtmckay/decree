# Machines


Every state does one thing: it **invokes a function**, and the function's result is an event that picks the next state. A function is either a script, or one of decree's three built-in decision functions. As far as the machine is concerned, they are all just functions.

## Example: the smallest machine

```yaml
# Graph: ../graph/hello.md
name: hello
description: Run one script.
initial: greet
states:
  greet:
    invoke: greet                  # runs scripts/greet (or scripts/hello/greet)
    transitions: { done: done }    # exit 0 -> done; non-zero -> failed, implicitly
  done:   { final: true }
  failed: { final: true }          # every machine has one
```

## Example: asking a person

```yaml
# Graph: ../graph/deploy.md
name: deploy
description: Build, ask a person to approve, then ship.
initial: build
states:
  build:
    invoke: build
    transitions: { done: approval }
  approval:                        # a person picks one option; the run pauses for the reply
    invoke: { choose: person, question: "Ship this build?", ask: ask_person, timeout_s: 86400 }
    transitions:
      approve: { target: ship, description: Ship this build. }
      reject:  { target: rejected, description: Do not ship. }
  ship:
    invoke: ship
    transitions: { done: done }
  done:     { final: true }
  rejected: { final: true }
  failed:   { final: true }
```

## Example: composing machines

A state can invoke a whole machine. It runs as a child run, and the final state it reaches becomes this state's event: `done`, `rejected`, and `failed` as `error`.

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

## Example: feature

Scripts, a deterministic check, a model's choice that hands over to a person when unsure, nesting, data and an emit.

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
    initial: implement
    transitions: { done.state.work: done }   # raised when work reaches its own final state
    states:
      implement:
        invoke: implement
        max_attempts: 2            # extension: re-run a crashing script in place
        onentry: [snapshot]
        onexit: [collect_logs]
        transitions: { done: verify }
      verify:                      # script: prints pass or fail
        invoke: verify
        transitions: { pass: verified, fail: rounds_left }
      rounds_left:                 # deterministic check: yes or no
        invoke: { check: { visits: implement, less_than: { data: max_rounds } } }
        transitions: { yes: triage, no: review }
      triage:                      # a model picks one option; below the floor it is unsure
        invoke: { choose: model, question: "Should we implement again or split the work?", min_confidence: 0.8, input: verify }
        transitions:
          retry:  { target: implement, description: The failures look fixable; implement again. }
          split:  { target: spawn_followups, description: The scope is too large; emit smaller follow-up messages. }
          unsure: { target: review }
      review:                      # a person picks one option; the run pauses for the reply
        invoke: { choose: person, question: "Tests still fail. What next?", ask: ask_person, timeout_s: 172800 }
        transitions:
          approve: { target: verified, description: Good enough; commit it. }
          retry:   { target: implement, description: Try again; see my note. }
          reject:  { target: failed, description: Stop. }
      verified: { final: true }    # raises done.state.work
  spawn_followups:
    invoke: spawn
    emits: [feature]               # extension: machines its scripts may emit messages for
    transitions: { done: done }
  done:   { final: true, onentry: [commit] }   # migrations: processed.md is written first, so the commit includes it
  failed: { final: true }
```

There are two retry mechanisms, and they mean different things. `max_attempts` re-runs a crashing script in place: same state, no `onexit` or `onentry`. A transition back to an earlier state (`retry` above) is a new round chosen by the machine, bounded here by the `rounds_left` check.

## Invoke: the state's function

`invoke` is SCXML's `<invoke>`. Invoking another state machine is SCXML's own invoke type; the others are decree's, as SCXML lets each platform define types.

| `invoke` | SCXML `type` | What runs | Events it produces |
| --- | --- | --- | --- |
| `<script name>` | `decree:script` | The script (section 6). | `done` (exit 0), `error` (non-zero), or an event the script prints. |
| `{ check: <condition>, input?: <state> }` | `decree:check` | decree evaluates the condition. Deterministic, no AI. | `yes` or `no`. |
| `{ choose: model, question: <text>, router?: <machine>, min_confidence?: <0..1>, input?: <state> }` | `decree:model` | A router machine (default: the machine named `router`) asks a model to pick one of the state's transitions (section 7). | One of the state's events; `unsure` if its confidence is below `min_confidence`; `error` if the router fails or replies with something that is not an option. |
| `{ machine: <name>, params?: {…} }` | `http://www.w3.org/TR/scxml/` (SCXML's own: a child state machine) | The machine runs as a child run (section 7, Sub-machines). | The id of the root final state the child reached; `failed` becomes `error`. |
| `{ choose: person, question: <text>, ask: <script>, timeout_s?: <int> }` | `decree:person` | The `ask` script tells someone; the run pauses until a reply picks one of the state's transitions (section 4, Replies). | One of the state's events; `error` on timeout. |

A state with no `invoke` and a `done` transition passes straight through (SCXML's eventless transition).

**Choices.** For `choose`, `question` is what is being decided, and the options are the state's transitions, except `unsure` and `error`. Each option must have a `description`. The question and the options with their descriptions are exactly what the model or the person sees, so write them to stand alone. They map directly onto decision models: TypeSafe Jev's `instructions` and `criteria`, GLiNER2's instructions and described labels, and the question-and-answers of OpenAI's Decisions API. Inside `{ … }`, quote questions and descriptions (`question: "Ship this build?"`): a `, ` or `?` in unquoted text breaks YAML's inline maps, and `decree check` then rejects the machine. The examples here quote every question for that reason.

**Input.** `input` names a state; `matches` and the model read that state's latest script output (stdout and stderr, as logged). Without `input`, they read the output of the most recent script in the run. A model also always sees the message body.

**Conditions** are typed objects, one subject and one operator, so the YAML parser and `decree check` catch mistakes:

| Condition | Meaning |
| --- | --- |
| `{ matches: '<regex>' }` | The input (a state's output) matches the regular expression. |
| `{ data: <name>, matches: '<regex>' }` | A `string` data value, such as a file name from the message's `params`, matches the regular expression. |
| `{ visits: <state>, <op>: <value> }` | How many times `<state>` has been entered in this run, compared to a value. |
| `{ data: <name>, <op>: <value> }` | A `data` value compared to a value. |
| `{ confidence: <state>, <op>: <number> }` | The confidence of the latest `choose: model` decision in `<state>`, 0 to 1 (0 if the router reported none). |

`<op>` is one of `equals`, `not_equals`, `less_than`, `at_most`, `more_than`, `at_least`. `<value>` is a literal or `{ data: <name> }`. Regular expressions use the `regex` crate's syntax and match anywhere unless anchored. There is no `and`/`or`: to test two things, use two `check` states in a row.

**Escalation** is a chain of states, cheapest first, each passing what it cannot decide to the next: `no` from a `check`, `unsure` from a `choose: model` below its `min_confidence`. A `confidence` check after `unsure` splits the rest into bands (worth asking a person, or not). The `sort_document` machine in `mock/` goes the whole way: file name, document text, a local classifier, a large model, a person.

## Keys

"SCXML" gives the element or attribute a key stands for; "extension" marks keys SCXML does not have.

| Where | Key | Type | SCXML | Meaning |
| --- | --- | --- | --- | --- |
| Root | `name` | string | `<scxml name>` | Must equal the file stem. `^[a-z][a-z0-9_]*$`. |
| Root | `description` | string | extension | Required. Used in prompts, `decree status` and graphs. |
| Root | `data` | map of name to `{type, default}` | `<datamodel><data id>` | Optional. `type` is `string`, `int` or `bool`. `default` is required and must match `type`. A message's `params` override the defaults; nothing else changes them. |
| Root | `onentry`, `onexit` | list of script names | extension (`<scxml>` has neither; they act like a top-level compound state around all others) | Optional. Root `onentry` runs once when the run starts (and again when `decree retry` continues it); root `onexit` runs once after a root final state is entered. |
| Root | `initial` | state id | `<scxml initial>` | Required. Must be a direct child in `states`. |
| Root | `states` | map of id to state | child `<state>` and `<final>` | Required. The map key is the state's `id`. |
| State | `final` | `true` | `<final>` | Marks a final state. Final states may only have `description`, `onentry` and `emits`. |
| State | `description` | string | extension | Optional. |
| State | `invoke` | script name or decision object | `<invoke type>` | Optional on atomic states (above). |
| State | `max_attempts` | int | extension | Optional on script states. Default 1. Section 6. |
| State | `timeout_s` | int | extension | Optional on script states (limit for the script). For `choose: person`, it goes inside `invoke`. |
| State | `onentry`, `onexit` | list of script names | `<onentry>`, `<onexit>` | Optional. Run every time this state is entered or exited. |
| State | `initial`, `states` | as root | `<state initial>`, child states | Present together on compound states, absent on all others. |
| State | `transitions` | map of event to target | `<transition event target type>` | Short form `event: target`, or long form `{target, description, type}`. Not allowed on final states. |
| State | `emits` | list of machine names | extension | Machines this state's scripts may emit messages for. Enforced by `decree emit`, drawn by `decree graph`. |

## SCXML subset

decree implements SCXML's semantics (the algorithm in Appendix D, "Algorithm for SCXML Interpretation") for exactly this subset:

| SCXML feature | In decree |
| --- | --- |
| `<scxml>`, `<state>` (atomic and compound), `<final>` (at any level), `initial` | Yes. |
| `<transition event target type>` on atomic and compound states | Yes. One transition per event per state (a map), always with a target. `type: internal` as in SCXML. |
| Event matching | SCXML's: a transition's event matches an event with the same name, or one that extends it after a `.` (`done.state` matches `done.state.work`). The active state's transitions are checked first, then each ancestor's, innermost first. |
| `done.state.<id>` | Yes: raised when a compound state's own final child is entered, and handled before anything else. |
| `<invoke type>` | Yes: one per atomic state: a child machine (SCXML's own type), or `decree:script`, `decree:check`, `decree:model` or `decree:person` (above). The invoked function's result is an event sent to the machine, as SCXML invoked services send events to their parent. Script completion is `done` (SCXML `done.invoke.<id>`); failure is `error` (SCXML `error.execution`). |
| External events | Yes: a `choose: person` invoke receives the reply as an external event (section 4, Replies). |
| `<onentry>`, `<onexit>` | Yes, as lists of scripts. |
| Eventless transitions | Only `done` on a state with no `invoke`. |
| Data model | A custom data model (SCXML allows these through the `datamodel` attribute): read-only typed `data`, plus `visits`. Conditions are evaluated by `check` invokes, not on transitions. |
| `cond` on transitions | No. A decision is a state of its own (a `check` or `choose` invoke), so every transition is unconditional. |
| `<parallel>`, `<history>` | No. A run is always in exactly one atomic state, which is what `events.jsonl` records. |
| `<send>`, `<raise>`, `<assign>`, `<script>`, `<if>`, `<foreach>`, `<log>`, `<cancel>`, `<donedata>` | No. A script's printed event replaces `<raise>`; `decree emit` replaces `<send>` to other sessions; `data` is read-only. |
| Targetless transitions | No. |
| `<onentry>`, `<onexit>` on `<scxml>` | Not in SCXML. decree's root `onentry`/`onexit` (Keys) are an extension that behaves like a top-level compound state around all others. |
| `<onexit>` on `<final>` | No (V7). A final state has `onentry` only. |
| Unhandled events | Differs. SCXML discards an event no transition matches; decree turns it into `error`, and an unhandled `error` goes to `failed` (Rules), so a run never stalls on an invoke result. |
| `error.execution` in `<onentry>` | As SCXML: the failing block stops, entry completes, then `error` is handled (section 6). One difference: a failing `onentry` on a root-level final state moves the run to `failed`, where SCXML would already have terminated. |
| Result of a child machine | Differs. SCXML sends `done.invoke.<id>` (with `<donedata>`); decree sends the name of the child's root final state, `failed` as `error` (Invoke). |
| XML | Never. YAML is the only syntax. |

Where SCXML leaves a choice to the platform, decree's choice is in this spec. Anything outside the subset fails validation with a message that names the SCXML feature and the decree alternative, for example `cond on a transition is not supported: make the decision a state with invoke: { check: ... }`.

## Rules

- **State ids are unique across the whole machine,** as SCXML requires. A target is always a bare state id.
- **Events.** Script events are described in section 6. Decision events are listed in the Invoke table. `done.state.<id>` is raised by decree. Event names match `^[a-z][a-z0-9_]*(\.[a-z0-9_]+)*$`. Events a script prints and `choose` options may not be `done`, `error`, `unsure`, or start with `done.` or `error.`; `yes` and `no` are free to use.
- **Selecting a transition.** For an event, check the current atomic state's `transitions`, then its parent's, up to the root, and take the first match (Event matching, above). Within one state, no transition's event may extend another's (`done` beside `done.state.work`): SCXML would pick by document order, which a YAML map does not keep (V21).
- **Unhandled events.** If `error` matches nothing, the target is the root-level final state `failed`. Every machine must have it. (Like a Step Functions `Catch` on `States.ALL`.) Any other event that matches nothing becomes `error`.
- **`visits.<state>`** is how many times that state has been entered in this run, derived from `events.jsonl` (section 7).
- **Exit and entry order** is SCXML's. The transition domain is the deepest compound state (or the root) that is a proper ancestor of both the source and the declared target; for a `type: internal` transition from a compound state to one of its descendants, it is the source itself. Exiting runs `onexit` from the innermost active state outward, stopping below the domain. Entering runs `onentry` from the outermost new state inward, then follows `initial` down to an atomic state. A self-transition therefore exits and re-enters its state.

## Rust types
## YAML




`decree check` runs every rule below, plus the message checks M1–M3, and warns (without failing) when `.decree/graph/` is missing or differs from what `decree graph` would write. `process` and `daemon` run V1–V21 at start; any failure stops startup. Each rule needs one passing and one failing fixture test. Error format: `<path relative to .decree/>: <state path or line>: <message>`.

| Rule | Check |
| --- | --- |
| V1 | `name` equals the file stem and matches `^[a-z][a-z0-9_]*$`. |
| V2 | State ids match the same pattern and are unique across the machine. |
| V3 | Root and every compound `initial` names a direct child. |
| V4 | Every transition target exists. |
| V5 | A root-level final state named `failed` exists. |
| V6 | Compound states have `initial` and `states`, and no `invoke`. |
| V7 | Final states have only `final`, `description`, `onentry` and `emits`. |
| V8 | Decision and sub-machine states cover their events: a `check` state handles `yes` and `no`; a `choose: model` state with `min_confidence` handles `unsure`; a `choose` state has a `question` and at least two options, each with a `description`; a `machine` state handles every root final state of the child except `failed`. |
| V9 | Every `input` names a state with a script invoke. A `matches` check without `input` is in a machine that has a script state before it on some path. |
| V10 | Every condition has exactly one subject (`matches`, `visits`, `data` or `confidence`) and, except for a bare `matches`, exactly one operator; `visits` names an atomic state; `confidence` names a `choose: model` state and compares to a number from 0 to 1; `data` names existing data, compared to a value of its type, or with `matches` to a regex when its type is `string`; every `matches` compiles as a regular expression. |
| V11 | Every state is reachable from root `initial`, and every non-final state can reach a root-level final state. |
| V12 | Every script name (`invoke`, `ask`, `onentry`, `onexit`, root `onentry`, `onexit`) resolves to exactly one executable file (section 6). |
| V13 | Every `emits` entry is an existing machine name. |
| V14 | Every `data` default matches its `type`. |
| V15 | Every compound state with a final child handles `done.state.<id>`, itself or through an ancestor, so the run cannot stall. |
| V16 | Every `router` and `machine` names an existing machine, and a machine named `router` exists if any `choose: model` names no router; `params` are valid for the child's `data`; `min_confidence` is between 0 and 1; `max_attempts` only on script states. |
| V17 | `type: internal` appears only on a compound state's transition whose target is one of its descendants. |
| V18 | Event names follow the Rules; no reserved name is a script-printed event or a `choose` option. |
| V19 | Nothing outside the SCXML subset: unknown keys fail with the name of the SCXML feature, when there is one, and the decree alternative. |
| V20 | No cycle of `machine` and `router` invokes: a machine never invokes itself, directly or through others. |
| V21 | Within one state, no transition's event equals another's followed by `.` and more (`done` and `done.state.work`), so at most one of a state's transitions matches any event. |
| M1 | Every pending migration (not in `processed.md`) parses, names a known machine in `machine:` (or `routine:`), and has valid `params` for that machine's `data`. |
| M2 | Every `inbox/*.md` passes the same checks. |
| M3 | Every `cron/*.md` passes the same checks, and its `cron:` expression parses. |

