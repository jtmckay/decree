---
machine: rust_develop
---
# 71: One shape per machine key

## Overview

The machine format has places where one idea takes several shapes, which makes machines harder for people and models to read and write:

- **`invoke` names its kind three ways:**
  - a bare string is a script;
  - `check:` and `machine:` are keys;
  - `choose: model` and `choose: person` are values, and each comes with its own set of sibling fields.
- **Where a check's text comes from is implicit.** `{ matches: re }` reads "the input": the `input:` state named beside `check:`, or, without one, whichever script ran last. `matches` is both a subject (that input) and an operator (`{ data: file, matches: re }`).
- **A check's result is `yes` or `no`,** where a reader expects true and false.
- **Function settings are split.** `max_attempts` and a script's `timeout_s` sit on the state, while a person's `timeout_s` sits inside `invoke`.
- **The examples write prose inside flow maps** (`{ question: "…?" }`). An unquoted `?` or `, ` there breaks YAML, so the docs have to warn about it.

This is a breaking change to machine files, made before 0.5 is released. No old shape is accepted. Each old shape fails `decree check` with a message that names the new one (V19). This is not a shim: nothing old is read or translated.

## The new shapes

**`invoke`** is a map with exactly one key, which names the kind. Its value is that kind's object. For `script` and `machine`, a bare name is short for `{ name: <name> }`, and `invoke: <script name>` is short for `invoke: { script: <script name> }`.

| Kind | Long form | Events |
| --- | --- | --- |
| `script` | `{ script: { name: <script>, max_attempts?: <int>, timeout_s?: <int> } }` | `done`, `error`, or a printed event |
| `check` | `{ check: <condition> }` | `true` or `false` |
| `model` | `{ model: { question: <text>, router?: <machine>, min_confidence?: <0..1>, output?: <state> } }` | an option, `unsure`, `error` |
| `person` | `{ person: { question: <text>, ask: <script>, timeout_s?: <int> } }` | an option, `error` on timeout |
| `machine` | `{ machine: { name: <machine>, params?: {…} } }` | the child's root final state, `failed` as `error` |

- **Script settings move inside `invoke`.** `max_attempts` (default 1) and `timeout_s` are no longer state keys.
- **A model sees only what it is given.** It is given the latest output of the `output` state, if one is named, and the message body. Without `output`, the request's `input` is empty. There is no fallback to "the most recent script".

**Conditions** have exactly one subject and one operator, always:

| Subject | Value | Operators |
| --- | --- | --- |
| `output: <state>` | That state's latest script output, as logged (stdout, and stderr lines with their `[stderr] ` prefix). The state must have a script invoke. | `matches` |
| `data: <name>` | A `data` value | `equals`, `not_equals`, `less_than`, `at_most`, `more_than`, `at_least`; `matches` for `string` data |
| `visits: <state>` | Times that atomic state was entered in this run | the comparison operators |
| `confidence: <state>` | The latest `model` decision's confidence in that state, 0 to 1 (0 if none was reported) | the comparison operators |

`<value>` is a literal or `{ data: <name> }`. `matches` is only an operator.

**Check events are `true` and `false`.** In YAML 1.2, `true:` and `false:` as map keys are booleans. decree reads a boolean key in `transitions` as the event name `true` or `false`, so `transitions: { true: a, false: b }` works without quotes. `yes` and `no` are ordinary event names.

**A state** has only structure: `description`, `invoke`, `onentry`, `onexit`, `transitions`, `emits`, `initial` and `states` (compound), and `final`. Final states keep the existing rule (V7).

**Style** for every machine in this repository: decision invokes, and anything holding prose (`question`, `description`), use block style. Short structural maps (`transitions: { done: verify }`, `{ final: true }`) may stay inline. Inside a state, keys go in this order: `description`, `invoke`, `onentry`, `onexit`, `transitions`, `emits` (then `initial`, `states` for compound states). Defaults are not written out (no `max_attempts: 1`). The docs no longer need the quoting warning; replace it with one sentence recommending block style for prose.

Example, the `feature` machine's decisions after the change:

```yaml
      rounds_left:
        invoke:
          check: { visits: implement, less_than: { data: max_rounds } }
        transitions: { true: triage, false: review }
      triage:
        invoke:
          model:
            question: Should we implement again or split the work?
            min_confidence: 0.8
            output: verify
        transitions:
          retry:  { target: implement, description: The failures look fixable; implement again. }
          split:  { target: spawn_followups, description: The scope is too large; emit smaller follow-up messages. }
          unsure: review
```

## Requirements

Read all of `docs/reference/`, `mock/README.md`, `tests/README.md`, then `src/machine.rs`, `src/machine/validate.rs`, `src/cond.rs`, `src/graph.rs` and `src/interpreter/decide.rs`.

1. **Parser and types.** Change the machine types and parsing to the shapes above. The interpreter's behaviour is unchanged apart from these:
   - check events are `true`/`false`;
   - `matches` reads the named `output` state;
   - a model with no `output` gets an empty `input`.

   Remove every type, field and code path the old shapes needed.
2. **Validation.** Update V8 (a `check` handles `true` and `false`), V9 (`output` names a state with a script invoke, for both a condition and `model`), V10, V16 (`max_attempts` and `timeout_s` only inside a `script` invoke) and V18. V19 rejects each old shape with a message that names the new one:
   - `choose` → `model:` or `person:`;
   - `input` → `output`;
   - a bare `matches` → `{ output: <state>, matches: … }`;
   - a state-level `max_attempts` or `timeout_s` → inside `invoke: { script: … }`;
   - `{ machine: x, params: … }` → `{ machine: { name: x, params: … } }`.

   Keep a pass and a fail case per rule in `tests/validation_test.rs`, and add a fail case for each old shape.
3. **Graph.** Notes and edge labels follow the new names (`check: output read_text matches '…'`, `model: router, min_confidence 0.8`, `person: ask_person`). Regenerate every committed `.decree/graph/` (mock, examples, this repository).
4. **Every machine moves to the new shapes and style:**
   - `mock/.decree/machines/`, `examples/*/.decree/machines/`, `src/templates/machines/`, `src/templates/router/router.yml`;
   - this repository's `.decree/machines/`, refreshed from what `decree init --ai claude` writes once the templates change.
5. **Mock runs and docs.**
   - Recorded `decision` events in `mock/.decree/runs/` (event `yes`/`no` → `true`/`false`, `condition` as written) and any `request.json` they affect, so `tests/mock_replay_test.rs` passes unchanged.
   - `mock/README.md`, `docs/reference/` (machines.md: examples, Invoke table, Conditions, Keys, Validation; runs.md: Check and Choose sections; graph.md), the decree skill (`src/templates/skills/decree/`) and `src/templates/help.txt`.
   - `docs/decisions.md`: a new entry for this change: one shape per key; check events `true`/`false`; explicit `output`; settings inside `invoke`. Cite serde's externally tagged enums and GitHub Actions' `uses`/`run` as prior art for key-tagged kinds.
6. Keep `tests/docs_test.rs` (machine examples byte-identical to the mock), `tests/mock_templates_test.rs` and every black-box test green. Change a black-box test only where it writes a machine or asserts a `yes`/`no` check event or a validation message, and list each such change.

- Only this migration's scope; migration 72 adds the JSON Schema.
- If the reference docs and the code disagree, or a case is not covered here, do not guess: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- Tests build their own `.decree/` in a temp directory and never touch this repository's `.decree/`, except the explicit refresh in requirement 4. No test calls a real LLM or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass.

## Files to Modify

- src/machine.rs, src/machine/validate.rs, src/cond.rs, src/graph.rs, src/interpreter/, src/runtime.rs (attempts and timeout from the invoke)
- mock/, examples/, src/templates/, .decree/machines/ and every .decree/graph/
- docs/reference/, docs/decisions.md, tests/

## Acceptance Criteria

- **Given** each old shape (`choose:`, `input:`, a bare `matches`, a state-level `max_attempts` or `timeout_s`, `{ machine: x, params }`)
  **When** `decree check` runs
  **Then** it fails V19 with a message that names the new shape

- **Given** `mock/`, every project in `examples/`, a fresh `decree init` for each `--ai`, and this repository
  **When** `decree check` and `decree graph` run
  **Then** check exits 0 and graph leaves the committed `.decree/graph/` unchanged

- **Given** a check with `transitions: { true: a, false: b }` written without quotes
  **When** the condition holds, and when it does not
  **Then** the run goes to `a`, and to `b`

- **Given** `rg -n 'choose:|input:|\byes\b|\bno\b' mock/.decree/machines examples/*/.decree/machines src/templates docs/reference`
  **When** it runs
  **Then** no machine or doc example uses an old shape (list any remaining hits and why they are not machine keys)
