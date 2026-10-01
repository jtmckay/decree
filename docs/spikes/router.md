# Spike: the router contract

**Status:** open. R1–R3, R5, R6 and R8 decided; R4, R7 and R9 designed and awaiting confirmation; R10 dropped (decision record below). **Blocks:** spec section 7 "Router (provisional)", tickets M3.2 and M3.3 (migrations 47 and 48, held in `batch-2/`), and the router logs in `mock/`. **Timebox:** 3 days.

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
| R1 | Wire protocol between decree and a backend | (a) 0.4.2's `{prompt}` command template; (b) JSON `RouterRequest` on the command's stdin, JSON `RouterReply` on its stdout; (c) built-in HTTP clients per vendor | (b). Any language, no HTTP dependency in decree, one adapter per backend of about 30 lines. Same shape as git credential helpers. |
| R2 | Config and selection | One `commands.ai_router`; or named backends (`routers: { chat: …, jev: … }`) with `router: <name>` on the state replacing `router: llm` | Named backends, if the benchmark shows different backends suit different states. Otherwise keep one. |
| R3 | Who renders a chat prompt | decree (puts `prompt` in the request) or the chat adapter | decree, so every chat backend sees the same tested prompt; typed backends ignore it. |
| R4 | Confidence in the machine language | Record only; or `min_confidence: <0..1>` on router states. Below it: take `default`, or take a declared event (for example `unsure`) | Gate, below-floor goes to a declared event, because "unsure" usually means "ask a human", not "fail". TypeSafe suggests 0.6 as a floor. |
| R5 | What "ask a human" means | A final state (`needs_review`) plus `decree retry --state <s>` by the human; or a new waiting status | The final state. No new concept: `retry` already continues a finished run at a chosen state. |
| R6 | What the backend sees | Machine and state descriptions, options, the invoke's last 50 stdout lines, the message body. Limits and truncation | Keep the fields. Set a byte budget and truncate the step output from the top. Jev's state budget is 32k tokens. |
| R7 | Data leaving the machine | Message bodies and step output go to a hosted API (Jev is hosted only) | Opt-in per backend in config; document it in `decree init` output. |
| R8 | Reproducibility | Log the request only as prompt text, or as the JSON `RouterRequest` too | JSON request in the router log, so any decision can be replayed against another backend (the deferred `decree replay`, Q7). |
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

### Decided

| # | Decision | Reason |
| --- | --- | --- |
| R1 | JSON `RouterRequest` on a router command's stdin, JSON `RouterReply` on its stdout. | Any language, no HTTP client in decree, one small adapter per backend. |
| R2 | Named routers in config; a decision names the one it uses. | Different decisions can use different backends (chat, Jev, self-hosted, human). |
| R3 | decree renders the chat prompt and passes it in the request as `prompt`. | One tested prompt for every chat backend; typed backends ignore it. |
| R8 | Every router log holds the full JSON request and reply. | Any decision can be replayed against another backend; this is also the benchmark harness. |

### R6: decided

The backend's request holds:

- the machine and state descriptions and the options, never truncated;
- the tail of the step's combined log (stdout plus `[stderr]` lines), cut from the top;
- the message body, cut from the end;
- a short history of this run: visits per state, and earlier decisions with their reasons.

Each named router sets a byte budget; anything cut is listed in the decision event as `truncated`. If the step's script wrote `$DECREE_RUN_DIR/context.md`, that file replaces the log tail. This is how a script chooses what the model sees, and it is the main defence against leaking secrets. decree does no redaction of its own.

### Decided: asking a person (R5)

decree has waiting states (spec section 5, Kinds of state; section 4, Events for waiting runs): a state with no `invoke` and no `done` pauses the run until a reply message delivers one of its events. decree does not know there is a question or a person; the state's `onentry` script asks however it likes, and `decree event` or any tool writes the reply. No `decree retry` is involved. `decree process` prints every waiting run with the commands to reply, and exits 0.

### Designed, awaiting confirmation: confidence bands (R4)

A decision acts on the band of the highest confidence floor it meets. To escalate, a band takes an event that leads to a waiting state.

```yaml
verify:
  invoke: verify
  decide:
    by: jev                          # a named router (R2)
    shape: choice
    default: ask                     # the router failed twice
    confidence:                      # highest floor met wins
      - { at_least: 0.85, take: choice }    # high: act on the backend's pick
      - { at_least: 0.6,  take: ask }       # medium: ask a person
      - { at_least: 0,    take: default }   # low: recorded in the decision event, then the default
  transitions:
    pass:  { target: verified, description: All acceptance criteria are met. }
    retry: { target: implement, description: Failures look fixable; implement again. }
    ask:   { target: review, description: Not sure; ask a person. }
review:                              # a waiting state, as in mock/.decree/machines/feature.yml
  description: A person sends approve, retry or reject.
  onentry: [ask_person]
  transitions: { approve: verified, retry: implement, reject: failed }
```

- `take` is `choice` (act on the pick), `default`, or an event in `transitions`. Events named in `confidence` are never offered to the backend.
- Floors must be in descending order and the last must be `0`, so every confidence lands in a band. A backend that reports no confidence counts as 0.

**R7: self-hosted.** Routers are configurable (R2), so self-hosting is a matter of adapters plus guidance. Running the model server is out of this project's scope. Candidates for the benchmark and the guide:

| Option | How it fits a typed choice | Notes |
| --- | --- | --- |
| [SGLang](https://docs.sglang.ai/frontend/choices_methods.html) `select` | Scores every declared option by normalized log-probability, so it yields a real distribution over the options. | Closest to Jev's Choice. GPU. |
| [vLLM](https://docs.vllm.ai/en/v0.11.0/serving/openai_compatible_server.html) OpenAI-compatible server, `structured_outputs: {choice: [...]}` | Output constrained to the options. Probabilities only approximated from token log-probs. | `guided_choice` is deprecated in favour of `structured_outputs`. GPU. |
| llama.cpp server or Ollama (0.12+) with a JSON-schema `enum` | Constrained to the options, runs on CPU. | Log-prob support is uneven, so confidence is weak. |
| NLI zero-shot classifier (e.g. DeBERTa via `transformers`) | One probability per label. | CPU, small. Short context (about 512 tokens) and weak on technical output. Hugging Face TEI does not serve the zero-shot pipeline. |

Leaning: SGLang as the reference self-hosted backend, Ollama as the CPU fallback without confidence gating.

**R9: AI is always explicit.** Rules:

- `decide:` is the only place in a machine where a model chooses. It replaces `router: llm` and `default`. People choose outside decree (R4 + R5). Every other decision is deterministic: exit codes, events printed by scripts, `cond`s (always deterministic: `data` and `visits` only), attempts and timeouts.
- `shape` declares the response:
  - `choice`: options are the `transitions` events; the reply is an event, plus probabilities and confidence if the backend reports them.
  - `yes_no`: needs `question:`; `transitions` must have exactly `yes` and `no` (plus optional `error`). The reply is p(yes) from 0 to 1; the event is `yes` when p ≥ 0.5; confidence is max(p, 1 − p).
  - `score`: needs `question:` and `levels:`, 2 to 10 ordered `{description, event}` entries (several levels may share an event). The reply is a probability per level and their weighted score; the event is the level at the rounded score; confidence comes from the backend.
- The `router` event becomes `decision`, carrying `by`, `shape`, the reply, the confidence and the band taken.
- `decree graph` marks every decided edge with who decides and in what shape, e.g. `pass (choice: jev)`, `yes (yes_no: local)`, and band-diverted edges with their floor, e.g. `review (jev confidence < 0.85)`. Unmarked edges are deterministic. Each decision state gets a note with its `by`, `shape` and bands.
- Spec gets a "Who decides" table listing every mechanism as deterministic, model or human, with its response shape.

### Dropped

**R10: detecting AI use inside scripts.** Not pursued (2026-10-01). decree marks where a model makes a routing decision (R9); what a script does internally is the script's business.
