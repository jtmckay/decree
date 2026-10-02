# Spike: the router contract

**Status:** closed 2026-10-02. The contract is in spec sections 3, 5 and 7: routers are machines. Tickets M3.2 and M3.3 (migrations 47 and 48) implement it. The benchmark in Method is an optional follow-up: it informs which router to recommend, not the contract.

## Why

A router state asks a model to pick one of the events a machine declares. The spec's draft (section 7) is shaped around chat models: decree renders a prompt, runs `commands.ai_router` with it, digs a JSON object out of the reply text, and asks again if that fails. It works, but it treats a closed choice as free-text generation and throws away any signal about how sure the model was.

Typed-choice decision models fit the problem more directly. TypeSafe's Jev ([docs](https://docs.typesafe.ai/introduction), [API](https://docs.typesafe.ai/api.md)) takes a state plus a *Choice* question with declared options (up to 255) and returns `{choice, probabilities, confidence}`. That is a router state almost exactly, and its confidence would let a machine stop or escalate instead of acting on a guess ([confidence routing](https://docs.typesafe.ai/patterns/confidence-routing.md)).

The spike decides how decree talks to router backends, so chat models and typed-choice models both fit, and whether confidence becomes part of the machine language.

## Not in scope (already settled)

These hold whatever the spike finds:

- A router state's options are its declared events, minus `error`, minus those whose `cond` is false.
- One remaining option is taken without asking.
- decree validates the chosen event against the options, asks at most twice, then takes `default`.
- Every decision is recorded as a `router` event in `events.jsonl`, with optional `confidence` and `probabilities` fields (spec section 7).
- The interpreter talks to a `Router` trait that takes a structured `RouterRequest` and returns a `RouterReply` (spec section 7). The spike decides what sits behind it.

## Questions to answer

| # | Question | Options | Leaning, to confirm |
| --- | --- | --- | --- |
| R1 | A router is a machine. decree writes `request.json`, runs the router as a child run, and reads `reply.json` (spec section 7). | Replaceable and visible like any machine; any backend, any language; no model code in decree. |
| R2 | `default_router: <machine>` in `config.yml`; `choose: model, router: <machine>` picks another. | Different decisions can use different routers. |
| R3 | The prompt lives in the default router's script (`claude_router`'s `ask_claude`), and the spec documents it. | Prompt changes are a script edit, not a decree release. |
| R4 | Confidence in the machine language | Record only; or `min_confidence: <0..1>` on router states. Below it: take `default`, or take a declared event (for example `unsure`) | Gate, below-floor goes to a declared event, because "unsure" usually means "ask a human", not "fail". TypeSafe suggests 0.6 as a floor. |
| R5 | What "ask a human" means | A final state (`needs_review`) plus `decree retry --state <s>` by the human; or a new waiting status | The final state. No new concept: `retry` already continues a finished run at a chosen state. |
| R6 | The request holds descriptions, options, the input state's output, the message body and the run's history; trimming to a model's budget is the router's job. A script can write `context.md` to choose what the model sees. | Bounded, explainable input; secrets stay out by design, not by redaction. |
| R7 | Self-hosting is a router machine. `docs/routers.md` (M3.3) covers Jev through an adapter, SGLang, Ollama as a CPU fallback, and a cheap-model-first escalation router. Running a model server is out of scope. | Keep decree small. |
| R8 | Each router run is its own child run with its logs, `request.json` and `reply.json`; every decision is a `decision` event linking to it. | Replay and audit. |
| R9 | Beyond Choice | Jev's Score and Noul (yes/no probability) could back conditions, for example "is this diff risky?" | Out of scope for 0.5.0. Note what it would take. |

## Method

1. **Dataset.** This repository has no recorded router decisions (every 0.4 migration named its routine), so build about 40 labelled cases by hand:
    - `feature.verify` (about 25): real failing `cargo test` output from `.decree/runs/*/test-output.log` and from the mock, each labelled `retry`, `split` or `fail` by a human.
    - `triage.classify` (about 15): short requests labelled `small_change`, `feature` or `reject`.
    - Store each case as a `RouterRequest` JSON file plus its label, under `docs/spikes/router/cases/`.
2. **Adapters.** Prototype three, as R1 (b) scripts under `docs/spikes/router/`, not shipped:
    - `chat-router`: renders the section 7 draft prompt and calls `claude -p`.
    - `local-router`: the self-hosted candidate chosen in R7 (SGLang `select` or vLLM `structured_outputs.choice`).
    - `jev-router`: maps the request to one Choice question and calls `POST https://api.typesafe.ai/v1/systemone` with `model: jev-latest`. The exact question JSON comes from the API reference; the sketch below is a guess to be corrected.
3. **Harness.** A script that feeds every case to each adapter three times and records the reply, latency and cost.
4. **Measure.** For each backend:
    - accuracy against the labels
    - invalid replies (the second ask, or a `default` fallback)
    - agreement across the three repeats
    - latency p50 and p95
    - cost per decision
    - for Jev, accuracy per confidence bucket, to see whether a floor such as 0.6 separates right from wrong
5. **Decide** R1 to R9 and write the decision record below.

## Draft shapes, for evaluation

Request decree would write to a router command's stdin (R1 b):

```json
{
  "v": 1,
  "machine": "feature",
  "machine_description": "Implement one feature spec with an AI agent, verify it, and commit.",
  "state": "verify",
  "state_description": "Tests and acceptance criteria have run; decide what happens next.",
  "options": [
    {"event": "fail", "description": "Not fixable automatically."},
    {"event": "pass", "description": "All acceptance criteria are met."},
    {"event": "retry", "description": "Failures look fixable; implement again."},
    {"event": "split", "description": "Scope is too large; emit smaller follow-up messages."}
  ],
  "step_output": "running 14 tests\n...\ntest result: FAILED. 13 passed; 1 failed\n",
  "message_body": "# Rate-limit /api/upload\n...",
  "previous_error": null
}
```

Reply read from its stdout:

```json
{"event": "retry", "reason": "One test fails on an off-by-one; the fix is local.", "confidence": 0.83, "probabilities": {"fail": 0.05, "pass": 0.02, "retry": 0.83, "split": 0.10}}
```

The Jev call a `jev-router` adapter would make. Field names follow the API overview (`model`, `state`, `questions`, answers with `choice`, `probabilities`, `confidence`), but the option format is unconfirmed:

```json
{
  "model": "jev-latest",
  "state": {"workflow": "feature: Implement one feature spec…", "step": "verify: Tests and acceptance criteria have run…", "step_output": "…", "task": "…"},
  "questions": {
    "next": {"type": "choice", "question": "What should happen next?", "options": {"fail": "Not fixable automatically.", "pass": "All acceptance criteria are met.", "retry": "Failures look fixable; implement again.", "split": "Scope is too large; emit smaller follow-up messages."}}
  }
}
```

For how a machine declares a decision, confidence gating and human escalation, see the R4, R5 and R9 designs in the decision record below.

## Done when

- R1 to R9 each have a decision and a one-line reason in the record below.
- Spec section 7 has no "provisional" marker, and `commands.ai_router` either stays or is replaced in section 3.
- Migrations 47 and 48 are rewritten from the decision and moved back into `.decree/migrations/`.
- `docs/routers.md` exists: how to configure each backend, and how to set up a self-hosted one (R7).
- The router logs and router events in `mock/` match the decided contract.

## Decision record

The decision model changed during the spike. A decision is no longer a special kind of state with a router; it is a function a state invokes, like a script (spec section 5, Invoke): `check` (deterministic), `choose: model` or `choose: person`. Each option is one of the state's transitions, with a description.

| # | Decision | Reason |
| --- | --- | --- |
| R1 | A router is a command: it reads the rendered prompt (`input: prompt`) or the JSON request (`input: request`) on stdin and prints a reply. | Any language and any backend; no HTTP client in decree. |
| R2 | Named `routers:` in `config.yml`, plus `default_router`; `choose: model, router: <name>` picks one. | Different decisions can use different backends. |
| R3 | decree renders the chat prompt (spec section 7, Prompt template). | One tested prompt for every chat backend. |
| R4 | `min_confidence` on `choose: model`; below it the state produces `unsure`, an ordinary event. Escalating to a person is a transition from `unsure` to a `choose: person` state. No bands. | The threshold is one number where the decision is made; escalation is visible in the machine and the graph. |
| R5 | `choose: person`: the `ask` script asks; the run pauses; a reply message delivers one option. | decree never knows there is a question; any tool can answer; no `decree retry`. |
| R6 | The request holds descriptions, options, the input state's output (cut from the top), the message body (cut from the end) and the run's history, within the router's `max_input_bytes`; cuts are recorded. A script can write `context.md` to choose what the model sees. | Bounded, explainable input; secrets stay out by design, not by redaction. |
| R7 | Self-hosting is a router configuration. `docs/routers.md` (M3.3) covers a chat CLI, Jev through an adapter, SGLang, and Ollama as a CPU fallback. Running a model server is out of scope. | Keep decree small. |
| R8 | Every model call logs the full request and reply; every decision is a `decision` event. | Replay and audit. |
| R9 | AI and people appear only where a machine says `choose`; deterministic decisions are `check`. Graph edges say who decided: `(check)`, `(model)`, `(person)`. | It is always clear when an AI is involved. Typed conditions, not expression strings, keep YAML readable and checkable. |
| R10 | Not pursued: detecting AI use inside scripts. | What a script does internally is the script's business. |

## Research: decision models (2026-10-02)

Checked so that the request and reply contract fits real decision backends.

| | Input | Output | Runs | Sources |
| --- | --- | --- | --- | --- |
| TypeSafe Jev, Choice | `state` (any JSON), `instructions` (the question), `criteria` (option name to description); up to 255 options; several questions per call | `choice`, `probabilities` (sum to 1), `confidence` (from the distribution's shape) | Hosted only; `POST https://api.typesafe.ai/v1/systemone` | [Choice](https://docs.typesafe.ai/primitives/choice.md), [API](https://docs.typesafe.ai/api.md) |
| Fastino GLiNER2.5-Decide (1B) | Text; labels, optionally with descriptions; optional instructions; yes/no as a two-label task | The label; per-label confidence with `include_confidence=True` | Local on CPU or GPU (`pip install gliner2`, Apache-2.0); also Fastino's cloud API | [Model card](https://huggingface.co/fastino/GLiNER2.5-Decide-1B), [GLiNER2](https://github.com/fastino-ai/GLiNER2) |
| OpenAI Decisions API | Context (text or images); questions with fixed answer lists | One answer per question; confidence reported in coverage, not documented | Limited preview since 2026-09-29; no schema, endpoint or pricing published as of 2026-09-30 | [Overview](https://www.firecrawl.dev/blog/openai-decisions-api-vs-jev) |

What it changed:

- **`question` is required on `choose`.** All three take a question; the state description was optional and often empty. It maps to Jev's `instructions`, GLiNER2's instructions, and the Decisions API's question.
- **Options keep their descriptions.** Jev's `criteria` and GLiNER2's described labels both use them, and GLiNER2 in particular reads meaning from label text: an adapter should pass descriptions, not just terse event names.
- **`input` and `message_body` stay separate** in the request, so an adapter can send structured context (Jev's `state` takes any JSON).
- **Confidence is per router.** Jev's comes from its distribution, GLiNER2's is a per-label score, chat models self-report. `min_confidence` must be calibrated for the router it is used with; the `decision` event records the router.
- **GLiNER2.5-Decide is the self-hosted answer to R7:** a small CPU model built for this, better suited than a general LLM. Load it once in a long-running local server; do not start Python per decision.

Not covered yet: images as context (the Decisions API accepts them). The request is versioned (`v: 1`), so an `attachments` field can be added later without breaking routers.
