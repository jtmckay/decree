---
machine: rust_develop
---
# 88: Attempt lists on script invokes: `attempts` replaces `max_attempts`

## Summary

A `script` invoke lists its attempts. decree runs the script once per entry, in order, and stops at
the first attempt that does not end in `error`. Each attempt gets its entry in `DECREE_ATTEMPT_VALUE`,
so the script can choose what does the work: a local model, then a hosted one, then local again.

`attempts` replaces `max_attempts`. The list is the whole retry policy: its length is the limit,
and its entries say what each attempt is.

```yaml
states:
  implement:
    invoke:
      script: { name: implement, attempts: [local, local, claude, local], timeout: 3h }
    transitions: { done: verify, error: ask_person }   # all four failed: a person
```

`implement` runs with `DECREE_ATTEMPT_VALUE=local`. If it fails, it runs again with `local`, then
`claude`, then `local`. The first attempt that ends in anything other than `error` decides the
state's event. If all four fail, the event is `error`, and the state's transitions take it as today.

A plain retry with no value keeps a short form: `attempts: 3` means three attempts with
`DECREE_ATTEMPT_VALUE` unset. This is the same pattern as `invoke: name` for `invoke: { script: name }`.

## Motivation

Today, running the same step with a stronger model needs one state per model and a naming
convention the script parses (`implement`, `implement_claude`), or an escalation machine that
counts failures in a file. `max_attempts` can only repeat the same thing. An attempt list says the
whole policy in one place, `decree check` can validate it, and the graph can show it.

Prior art: fallback lists in model gateways (LiteLLM `fallbacks`, OpenRouter `models`) try each
entry in order until one succeeds. Step Functions `Retry` and Temporal's `RetryPolicy` cap attempts
with a number. A list does both.

## Specification

### Machine syntax

- **`attempts`** on a `script` invoke is either:
  - a list of 1 or more strings, each matching `^[A-Za-z0-9][A-Za-z0-9._:/@-]{0,127}$`, so model
    ids such as `claude-opus-5-5` or `qwen3:8b` fit; repeats are allowed; or
  - a positive integer *n*: *n* attempts with no value.
- The default is one attempt with no value, as `max_attempts` defaults to 1 today. Defaults are not
  written out: `attempts: 1` is valid but unidiomatic, like `max_attempts: 1` today.
- **`max_attempts` is removed.** It fails with V19, naming the replacement:
  `max_attempts: 3` → `attempts: 3`.
- `attempts` is valid only inside a `script` invoke, as `max_attempts` is today. That excludes
  `onentry`/`onexit`, `check`, `model`, `person` and `machine`.

### Running the attempts

Within one visit:

| Attempt *k* ends in | Result |
| --- | --- |
| `done`, or any event the script names | That is the state's event. No more attempts. |
| `error` (non-zero exit or timeout), with attempts left | Attempt *k*+1 runs in place, as `max_attempts` does today. |
| `error` on the last attempt | The state's event is `error`. |

- Attempts run **in place**: no `onexit`/`onentry`, and `visits` does not change. This is
  unchanged from `max_attempts`.
- `timeout` applies to each attempt.
- A new visit starts again at attempt 1. A script that should go straight to a stronger model on a
  later round reads `DECREE_VISITS`.
- `decree process --retry` re-enters the state, which is a new visit, so it starts at attempt 1.

### Script environment

| Variable | Value |
| --- | --- |
| `DECREE_ATTEMPT` | Unchanged: the attempt number in this visit, from 1. |
| `DECREE_ATTEMPT_VALUE` | This attempt's list entry, e.g. `claude`. Unset with the integer form, and for `onentry`/`onexit` scripts. |
| `DECREE_ATTEMPT_VALUES` | The whole list, space-separated. Unset with the integer form. |
| `DECREE_MAX_ATTEMPTS` | The number of attempts: the list's length, or the integer. |
| `DECREE_FINAL_ATTEMPT` | Unchanged: `true` on the last attempt. |

### Events (`events.jsonl`)

- **`script` events** gain `attempt_value` (string) when the attempt has one.
- **`transition` events with `source: "attempt"`** gain `attempt_value`: the value of the attempt
  about to run.
- `events.schema.json` documents both. Older runs stay valid, since the fields are only added.

### `decree check`

- **V16:** the reference to `max_attempts` becomes `attempts`.
- **V19:** `max_attempts` anywhere fails with ``write `attempts: <n>` or `attempts: [<value>, …]` ``.
- **New rule:** `attempts` is a positive integer or a non-empty list of valid values. The error
  names the state and the bad entry.

### Graph (`decree graph`)

A state with a list gets the note `attempts: local → local → claude → local`. The integer form
gets no note, as `max_attempts` gets none today.

### Built-in machines, docs and schemas

- `machine.schema.json`: `attempts` as `oneOf` integer ≥ 1 or a list of strings with the pattern
  above, with a description and an example of each. `max_attempts` is removed.
- `events.schema.json`: `attempt_value`.
- `docs/reference/machines.md`:
  - the invoke table, root-key table and style note;
  - "Two kinds of retry" uses `attempts`, with an example list and when to use a list instead of
    a transition to a different state.
- `docs/reference/scripts.md`: the two new variables.
- `src/templates`: every built-in machine, the skill and every example uses `attempts`.
- `CHANGELOG.md`: under Changed, `max_attempts` replaced by `attempts`.
- Nothing accepts `max_attempts` any more. 0.5 is in beta, so no alias.

## Not in scope

- An `escalate` event, root `tiers`, `reentry` and a `tier` condition (this file's earlier draft).
  A script that cannot do the work with its value exits non-zero, and the next attempt runs.
  A later round that should start higher reads `DECREE_VISITS`.
- Choosing the list from message `params` or `data`.
- Attempt lists on `model` invokes (router fallback). A separate feature, if ever.

## Acceptance criteria

- **Given** a state with `attempts: [local, claude]` and a script that exits 1 when
  `DECREE_ATTEMPT_VALUE=local` and 0 when it is `claude`
  **When** the run reaches the state
  **Then** the script runs twice, with values `local` then `claude` and `DECREE_ATTEMPT` 1 then 2,
  the state takes `done`, and `visits` for the state is 1

- **Given** `attempts: [local, local, claude, local]` and a script that always exits 1
  **When** the run reaches the state
  **Then** the script runs four times with those values in order, and the state's event is `error`

- **Given** `attempts: [local, claude]` and a script that names `fail` on its first attempt
  **When** it runs
  **Then** the state takes `fail`, and the script does not run with `claude`

- **Given** `attempts: 3`
  **When** the script fails every time
  **Then** it runs three times with `DECREE_ATTEMPT_VALUE` unset and `DECREE_MAX_ATTEMPTS=3`

- **Given** a machine with `max_attempts: 2`
  **When** `decree check` runs
  **Then** it exits 1 with a V19 error saying to write `attempts: 2`

- **Given** `attempts: []`, `attempts: 0`, an entry with a space, or `attempts` on a `model` invoke
  **When** `decree check` runs
  **Then** it exits 1, with an error naming the state

- **Given** a run with a list
  **When** it finishes
  **Then** each `script` event has the `attempt_value` it ran with, and each
  `source: "attempt"` transition has the value of the attempt it started

- **Given** a state with `attempts: [local, claude]` whose first visit ended with `claude`
  **When** a transition enters it again
  **Then** the first attempt of the new visit runs with `local`

- **Given** the repository
  **When** `rg -n max_attempts --glob '!.decree/migrations/**' --glob '!.decree/runs/**'` runs
  **Then** nothing matches, except the V19 message, its test and the CHANGELOG

- Only this migration's scope. Convert every machine that uses `max_attempts` (built-in templates, examples, docs, the skill, tests, this repository's own `.decree/machines/`) to `attempts`, unchanged in meaning: `max_attempts: 3` becomes `attempts: 3`. Do not redesign any machine here; migration 89 does that for the examples.
- Do not edit `.decree/migrations/` or `.decree/runs/`.
- If the reference docs and this migration disagree in a way you cannot settle, write the question to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- No test calls a real LLM or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass, and `decree check` passes in every example directory.
