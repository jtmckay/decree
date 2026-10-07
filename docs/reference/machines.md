# Machines

A machine is a YAML statechart in `.decree/machines/<name>.yml`: an SCXML document written as YAML. It holds only structure: states, what each state invokes, and transitions from events to states. It never contains code or paths. Keys use SCXML's names (`initial`, `states`, `transitions`, `target`, `type`, `onentry`, `onexit`, `invoke`, `data`, `final`); a few decree extensions are marked as such.

Every state does one thing: it **invokes a function**, and the function's result is an event that picks the next state. A function is a script, one of decree's three built-in decision functions (`check`, `model`, `person`), or another machine. As far as the machine is concerned, they are all just functions.

## Example: the smallest machine

```yaml
# yaml-language-server: $schema=../schema/v1/machine.schema.json
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
# yaml-language-server: $schema=../schema/v1/machine.schema.json
# Graph: ../graph/deploy.md
name: deploy
description: Build, ask a person to approve, then ship.
initial: build
states:
  build:
    invoke: build
    transitions: { done: approval }
  approval:                        # a person picks one option; the run pauses for the reply
    invoke:
      person:
        question: Ship this build?
        ask: ask_person
        timeout: 1d
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
# yaml-language-server: $schema=../schema/v1/machine.schema.json
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
# yaml-language-server: $schema=../schema/v1/machine.schema.json
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
        invoke:                    # attempts: re-run a crashing script in place
          script: { name: implement, attempts: 2 }
        onentry: [snapshot]
        onexit: [collect_logs]
        transitions: { done: verify }
      verify:                      # script: names pass or fail
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
            timeout: 2d
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

There are two retry mechanisms, and they mean different things. `attempts`, inside the script invoke, re-runs a crashing script in place: same state, no `onexit` or `onentry`. A transition back to an earlier state (`retry` above) is a new round chosen by the machine, bounded here by the `rounds_left` check.

`attempts` is a number or a list. `attempts: 2` runs the script up to twice with `$DECREE_ATTEMPT_VALUE` unset. A list gives each attempt a value, in order, and its length is the limit, as a model gateway's fallback list (LiteLLM `fallbacks`, OpenRouter `models`):

```yaml
implement:
  invoke:
    script: { name: implement, attempts: [local, local, claude, local], timeout: 3h }
  transitions: { done: verify, error: ask_person }   # all four failed: a person
```

`implement` runs with `DECREE_ATTEMPT_VALUE=local`; if it exits non-zero or times out, it runs again with `local`, then `claude`, then `local`. The first attempt that ends in anything but `error` (`done`, or an event the script names) is the state's event; if all four fail, the event is `error`. Each entry matches `^[A-Za-z0-9][A-Za-z0-9._:/@-]{0,127}$`, so model ids such as `claude-opus-5-5` or `qwen3:8b` fit, and repeats are allowed. A new visit starts again at attempt 1; a script that should start higher on a later round reads `$DECREE_VISITS`.

Use a list when the same step, with the same inputs and the same next state, is only done by something else: a stronger model, another host. Use a transition to a different state when what happens next differs: another script, a person, a step whose output the machine branches on.

## Invoke: the state's function

`invoke` is SCXML's `<invoke>`. Invoking another state machine is SCXML's own invoke type; the others are decree's, as SCXML lets each platform define types.

`invoke` is a map with exactly one key, which names the kind; its value is that kind's object. This is how serde writes an externally tagged enum, and how GitHub Actions tells a step's `uses` from its `run`. For `script` and `machine`, a bare name is short for `{ name: <name> }`, and `invoke: <script name>` is short for `invoke: { script: <script name> }`.

| `invoke` | SCXML `type` | What runs | Events it produces |
| --- | --- | --- | --- |
| `script: { name: <script>, attempts?: <int> \| [<value>, …], timeout?: <duration> }` | `decree:script` | The script ([scripts.md](scripts.md)), once per attempt until one does not end in `error` (default 1 attempt; a list gives each its `$DECREE_ATTEMPT_VALUE`), each stopped after `timeout` ([Durations](#durations), [Execution](scripts.md#execution)). | `done` (exit 0), `error` (non-zero), or the event the script names in `$DECREE_EVENT_FILE`. |
| `check: <condition>` | `decree:check` | decree evaluates the condition. Deterministic, no AI. | `true` or `false`. |
| `model: { question: <text>, router?: <machine>, min_confidence?: <0..1>, output?: <state> }` | `decree:model` | A router machine (default: the machine named `router`) asks a model to pick one of the state's transitions ([Model](runs.md#model)). | One of the state's events; `unsure` if its confidence is below `min_confidence`; `error` if the router fails or replies with something that is not an option. |
| `person: { question: <text>, ask: <script>, timeout?: <duration> }` | `decree:person` | The `ask` script tells someone; the run pauses until a reply picks one of the state's transitions, or until `timeout` passes ([Durations](#durations), [Replies](messages.md#replies)). | One of the state's events; `error` on timeout. |
| `machine: { name: <machine>, params?: {…} }` | `http://www.w3.org/TR/scxml/` (SCXML's own: a child state machine) | The machine runs as a child run ([Sub-machines](runs.md#sub-machines)). | The id of the root final state the child reached; `failed` becomes `error`. |

A state with no `invoke` and a `done` transition passes straight through (SCXML's eventless transition).

**Choices.** For `model` and `person`, `question` is what is being decided, and the options are the state's transitions, except `unsure` and `error`. Each option must have a `description`. The question and the options with their descriptions are exactly what the model or the person sees, so write them to stand alone. They map directly onto decision models: TypeSafe Jev's `instructions` and `criteria`, GLiNER2's described labels, and the question-and-answers of OpenAI's Decisions API. Write prose (`question`, `description`) in block style, one key per line, so no character in it can end a YAML flow map.

**Output.** `output` names a state with a script invoke; a `matches` condition and a model read that state's latest script output, as logged (stdout, and stderr lines with their `[stderr] ` prefix). A model sees only what it is given: that output, if `output` is named, and the message body. Without `output`, the request's `input` is empty.

**Conditions** are typed objects with exactly one subject and one operator, so the YAML parser and `decree check` catch mistakes:

| Subject | Value | Operators |
| --- | --- | --- |
| `output: <state>` | That state's latest script output, as logged (Output, above). The state must have a script invoke. | `matches` |
| `data: <name>` | A `data` value. | `equals`, `not_equals`, `less_than`, `at_most`, `more_than`, `at_least`; `matches` for `string` data |
| `visits: <state>` | How many times that atomic state was entered in this run. | the comparison operators |
| `confidence: <state>` | The latest `model` decision's confidence in that state, 0 to 1 (0 if the router reported none). | the comparison operators |

For example `{ output: read_text, matches: '(?i)invoice' }`, `{ data: file, matches: '\.pdf$' }`, `{ visits: implement, less_than: { data: max_rounds } }` or `{ confidence: big_model, at_least: 0.4 }`. The comparison operators are `equals`, `not_equals`, `less_than`, `at_most`, `more_than` and `at_least`; `matches` is only an operator. `<value>` is a literal or `{ data: <name> }`. Regular expressions use the `regex` crate's syntax and match anywhere unless anchored. There is no `and`/`or`: to test two things, use two `check` states in a row.

**Escalation** is a chain of states, cheapest first, each passing what it cannot decide to the next: `false` from a `check`, `unsure` from a `model` below its `min_confidence`. A `confidence` check after `unsure` splits the rest into bands (worth asking a person, or not). The `sort_document` machine in [`examples/sort-documents/`](../../examples/sort-documents/README.md) goes the whole way: file name, document text, a local classifier, a large model, a person.

## Durations

A duration is a whole number of at most 9 digits followed by one unit: `s` (seconds), `m` (minutes), `h` (hours) or `d` (days), such as `90s`, `10m`, `12h` or `7d`. There are no fractions (`1.5h`), no combinations (`1h30m`), no other units (`1w`), no sign (`-1m`) and no bare numbers (`10`); write `90m` or `5400s` instead. It is the format of Kubernetes and Go durations (`time.ParseDuration`), restricted to one whole-number term. A machine's `timeout` and the `decree prune --older-than` and `decree daemon --interval` flags ([cli.md](cli.md)) all take it, through one parser.

## Keys

"SCXML" gives the element or attribute a key stands for; "extension" marks keys SCXML does not have.

| Where | Key | Type | SCXML | Meaning |
| --- | --- | --- | --- | --- |
| Root | `name` | string | `<scxml name>` | Must equal the file stem. `^[a-z][a-z0-9_]*$`. |
| Root | `description` | string | extension | Required. Used in prompts, `decree status` and graphs. |
| Root | `data` | map of name to `{type, default}` | `<datamodel><data id>` | Optional. `type` is `string`, `int` or `bool`. `default` is required and must match `type`. A message's `params` override the defaults; nothing else changes them. |
| Root | `onentry`, `onexit` | list of script names | extension (`<scxml>` has neither; they act like a top-level compound state around all others) | Optional. Root `onentry` runs once when the run starts (and again when `decree process --retry` continues it); root `onexit` runs once after a root final state is entered. |
| Root | `initial` | state id | `<scxml initial>` | Required. Must be a direct child in `states`. |
| Root | `states` | map of id to state | child `<state>` and `<final>` | Required. The map key is the state's `id`. |
| State | `final` | `true` | `<final>` | Marks a final state. Final states may only have `description`, `onentry` and `emits`. |
| State | `description` | string | extension | Optional. |
| State | `invoke` | script name, or a map with one key: `script`, `check`, `model`, `person` or `machine` | `<invoke type>` | Optional on atomic states (above). `attempts` and `timeout` go inside it. |
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
| `<invoke type>` | Yes: one per atomic state, its kind the key under `invoke`: a child machine (SCXML's own type), or `decree:script`, `decree:check`, `decree:model` or `decree:person` (above). The invoked function's result is an event sent to the machine, as SCXML invoked services send events to their parent. Script completion is `done` (SCXML `done.invoke.<id>`); failure is `error` (SCXML `error.execution`). |
| External events | Yes: a `person` invoke receives the reply as an external event ([Replies](messages.md#replies)). |
| `<onentry>`, `<onexit>` | Yes, as lists of scripts. |
| Eventless transitions | Only `done` on a state with no `invoke`. |
| Data model | A custom data model (SCXML allows these through the `datamodel` attribute): read-only typed `data`, plus `visits`. Conditions are evaluated by `check` invokes, not on transitions. |
| `cond` on transitions | No. A decision is a state of its own (a `check`, `model` or `person` invoke), so every transition is unconditional. |
| `<parallel>`, `<history>` | No. A run is always in exactly one atomic state, which is what `events.jsonl` records. |
| `<send>`, `<raise>`, `<assign>`, `<script>`, `<if>`, `<foreach>`, `<log>`, `<cancel>`, `<donedata>` | No. A script's named event replaces `<raise>`; `decree emit` replaces `<send>` to other sessions; `data` is read-only. |
| Targetless transitions | No. |
| `<onentry>`, `<onexit>` on `<scxml>` | Not in SCXML. decree's root `onentry`/`onexit` (Keys) are an extension that behaves like a top-level compound state around all others. |
| `<onexit>` on `<final>` | No (V7). A final state has `onentry` only. |
| Unhandled events | Differs. SCXML discards an event no transition matches; decree turns it into `error`, and an unhandled `error` goes to `failed` (Rules), so a run never stalls on an invoke result. |
| `error.execution` in `<onentry>` | As SCXML: the failing block stops, entry completes, then `error` is handled ([Events from an invoke](scripts.md#events-from-an-invoke)). One difference: a failing `onentry` on a root-level final state moves the run to `failed`, where SCXML would already have terminated. |
| Result of a child machine | Differs. SCXML sends `done.invoke.<id>` (with `<donedata>`); decree sends the name of the child's root final state, `failed` as `error` (Invoke). |
| XML | Never. YAML is the only syntax. |

Where SCXML leaves a choice to the platform, decree's choice is in this reference. Anything outside the subset fails validation with a message that names the SCXML feature and the decree alternative, for example `cond on a transition is not supported: make the decision a state with invoke: { check: ... }`.

## Rules

- **State ids are unique across the whole machine,** as SCXML requires. A target is always a bare state id.
- **Events.** Script events are described in [Events from an invoke](scripts.md#events-from-an-invoke). Decision events are listed in the Invoke table. `done.state.<id>` is raised by decree. Event names match `^[a-z][a-z0-9_]*(\.[a-z0-9_]+)*$`. Events a script names and `model` or `person` options may not be `done`, `error`, `unsure`, or start with `done.` or `error.`; `true`, `false`, `yes` and `no` are ordinary event names.
- **Selecting a transition.** For an event, check the current atomic state's `transitions`, then its parent's, up to the root, and take the first match (Event matching, above). Within one state, no transition's event may extend another's (`done` beside `done.state.work`): SCXML would pick by document order, which a YAML map does not keep (V21).
- **Unhandled events.** If `error` matches nothing, the target is the root-level final state `failed`. Every machine must have it. (Like a Step Functions `Catch` on `States.ALL`.) Any other event that matches nothing becomes `error`.
- **`visits.<state>`** is how many times that state has been entered in this run, derived from `events.jsonl` ([Step loop](runs.md#step-loop), Visits).
- **Exit and entry order** is SCXML's. The transition domain is the deepest compound state (or the root) that is a proper ancestor of both the source and the declared target; for a `type: internal` transition from a compound state to one of its descendants, it is the source itself. Exiting runs `onexit` from the innermost active state outward, stopping below the domain. Entering runs `onentry` from the outermost new state inward, then follows `initial` down to an atomic state. A self-transition therefore exits and re-enters its state.


## YAML

Machines are YAML 1.2. In YAML 1.2, `true:` and `false:` as map keys are booleans; decree reads a boolean key in `transitions` as the event name `true` or `false`, so `transitions: { true: a, false: b }` works without quotes. YAML 1.1 tools (PyYAML, some editors) also read `on`, `yes` and `no` as booleans, the "Norway problem" `serde_norway` is named after. decree reads them as strings.

Unknown keys fail validation everywhere: at the root, in a state, a `data` entry, a long-form transition, every `invoke` object and every condition, so a misspelled key never falls back to another meaning (V19). A condition's `<value>` is a literal (int, float, string, bool) or `{ data: <name> }`; floats appear only in `confidence` conditions.

**Style.** Decision invokes, and anything holding prose (`question`, `description`), use block style. Short structural maps (`transitions: { done: verify }`, `{ final: true }`) may stay inline. Inside a state, keys go in this order: `description`, `invoke`, `onentry`, `onexit`, `transitions`, `emits`, then `initial` and `states` for a compound state. Defaults are not written out (no `attempts: 1`).

**Start simple.** Write the most naive machine that does the job: one state per script, in a straight line, ending in `done`. Add decisions, loops, `data`, `attempts`, `timeout`, child machines or `emits` only when someone asks for them or a run has shown they are needed, and say why in the state's comment. The cheapest fix comes first: change the script, then `attempts`, then a transition on an event the script names, then a `check`, a `model` or `person` decision, and last a child machine.

## Schema

`decree schema` writes a JSON Schema (draft 2020-12) for every file decree reads or writes into `.decree/schema/v1/` ([Schemas](README.md#schemas)): among them `machine.schema.json` for machine files and `message.schema.json` for message frontmatter ([messages.md](messages.md)). Their single source is compiled into decree, `decree init` writes them, and `decree check` warns when they are missing or differ from what `decree schema` would write. Every property has a description taken from this reference, and the schemas carry examples, so they are one precise contract for people, editors and models alike.

**Editors.** Every machine starts with a comment that points the YAML language server at the schema, above its `# Graph:` line:

```text
# yaml-language-server: $schema=../schema/v1/machine.schema.json
```

The path is relative to the machine file. VS Code with the YAML extension by Red Hat, and other editors that run the YAML language server, then complete keys, show each key's description on hover, and underline a misspelled key or a wrong value as you type. No setting is needed. The language server does not read Markdown, so message frontmatter is checked by `decree check`, or by any JSON Schema validator given `message.schema.json`.

**Models.** A model that writes or edits a machine reads `.decree/schema/v1/machine.schema.json` first, writes against it, and runs `decree check` after.

**What the schema checks** is everything about one file's shape: its keys and their types, required keys and unknown keys (V19, including each shape migration 71 replaced), each `invoke` kind and its short forms, transitions in short and long form (with `true:` and `false:` keys read as the event names), conditions with exactly one subject and one operator that subject takes and a value of the right type (V10), the name patterns (V1, V2, V18, script and machine names), the ranges of `min_confidence` and `confidence` values (V10, V16), the root-level final state `failed` (V5), which keys each kind of state may have (V6, V7, `type: internal` only on a compound state for V17), the options of a `model` or `person` state (at least two, each with a `description`, none reserved; V8, V18), `true` and `false` rather than `yes` and `no` on a `check` state (V8), and `data` defaults of their `type` (V14). A message has one of two shapes: a message that names its `machine`, or a reply with `to` and `event`.

**What only `decree check` checks** is what a schema of one file cannot see: whether a name resolves (`initial` V3, targets V4, `output` V9, the `data`, `visits` and `confidence` a condition names and the types it compares V10, scripts V12, `emits` V13, `router`, `machine` and `params` V16, a message's machine and params M1–M3), that `name` equals the file stem (V1), that state ids are unique across nesting levels (V2), reachability (V11), `done.state.<id>` handling (V15), events handled through an ancestor (V8), that an internal transition targets a descendant (V17), cycles (V20), overlapping events (V21), that a regular expression compiles in the `regex` crate's syntax (V10), that a cron file has a `cron:` expression that parses (M3), and that a value written `1.0` is not an int (JSON Schema counts it as an integer).

The schema never accepts a machine that `decree check` rejects for its shape. It is stricter than the parser in three corners, as this reference is: a string key holds a string (`description: 123` is not one), `final` is only ever `true`, and a final state has no `onexit`, `transitions` or `states` at all, not even empty ones.

## Validation

`decree check` runs every rule below, plus the message checks M1–M3, and warns (without failing) when `.decree/graph/` is missing or differs from what `decree graph` would write, or `.decree/schema/` from what `decree schema` would write. `process` and `daemon` run V1–V21 at start; any failure stops startup. Error format: `<path relative to .decree/>: <state path or line>: <message>`.

| Rule | Check |
| --- | --- |
| V1 | `name` equals the file stem and matches `^[a-z][a-z0-9_]*$`. |
| V2 | State ids match the same pattern and are unique across the machine. |
| V3 | Root and every compound `initial` names a direct child. |
| V4 | Every transition target exists. |
| V5 | A root-level final state named `failed` exists. |
| V6 | Compound states have `initial` and `states`, and no `invoke`. |
| V7 | Final states have only `final`, `description`, `onentry` and `emits`. |
| V8 | Decision and sub-machine states cover their events: a `check` state handles `true` and `false` (one still written with `yes` and `no` is told to rename them); a `model` state with `min_confidence` handles `unsure`; a `model` or `person` state has a `question` and at least two options, each with a `description`; a `machine` state handles every root final state of the child except `failed`. |
| V9 | Every `output`, in a condition or a `model`, names a state with a script invoke. |
| V10 | Every condition has exactly one subject (`output`, `data`, `visits` or `confidence`) and exactly one operator that subject takes; `visits` names an atomic state; `confidence` names a `model` state and compares to a number from 0 to 1; `data` names existing data, compared to a value of its type, or with `matches` to a regex when its type is `string`; every `matches` compiles as a regular expression. |
| V11 | Every state is reachable from root `initial`, and every non-final state can reach a root-level final state. |
| V12 | Every script name (`script`, `ask`, `onentry`, `onexit`, root `onentry`, `onexit`) resolves to exactly one executable file ([Resolution](scripts.md#resolution)); a `person` has an `ask` script. |
| V13 | Every `emits` entry is an existing machine name. |
| V14 | Every `data` default matches its `type`. |
| V15 | Every compound state with a final child handles `done.state.<id>`, itself or through an ancestor, so the run cannot stall. |
| V16 | Every `router` and `machine` names an existing machine, and a machine named `router` exists if any `model` names no router; `params` are valid for the child's `data`; `min_confidence` is between 0 and 1; every `timeout` is a [duration](#durations); `attempts` is a positive integer or a non-empty list of values that match `^[A-Za-z0-9][A-Za-z0-9._:/@-]{0,127}$`; `attempts` and `timeout` appear only inside a `script` invoke (and `timeout` inside a `person`), which the parser enforces with V19. |
| V17 | `type: internal` appears only on a compound state's transition whose target is one of its descendants. |
| V18 | Event names follow the Rules (a boolean key in `transitions` is the event `true` or `false`); no reserved name is a script-named event or a `model` or `person` option. |
| V19 | Nothing outside the SCXML subset: unknown keys fail with the name of the SCXML feature, when there is one, and the decree alternative. Each shape this format replaced fails with the one to write instead: `choose` (`model:` or `person:`), `input` (`output`), a bare `matches` (`{ output: <state>, matches: … }`), `max_attempts` anywhere (`attempts: <n>` or `attempts: [<value>, …]`, inside `invoke: { script: … }`), `timeout_s` on a state (inside `invoke: { script: … }`), `timeout_s` in an invoke (`timeout: <n>s\|m\|h\|d`) and `{ machine: x, params: … }` (`{ machine: { name: x, params: … } }`). |
| V20 | No cycle of `machine` and `router` invokes: a machine never invokes itself, directly or through others. |
| V21 | Within one state, no transition's event equals another's followed by `.` and more (`done` and `done.state.work`), so at most one of a state's transitions matches any event. |
| M1 | Every pending migration (not in `processed.md`) parses, names a known machine in `machine:`, and has valid `params` for that machine's `data`. |
| M2 | Every `inbox/*.md` passes the same checks. |
| M3 | Every `cron/*.md` passes the same checks, and its `cron:` expression parses. |
