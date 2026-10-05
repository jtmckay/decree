# Router machines

A `model` state asks a **router**: an ordinary machine that reads a request, asks a model however it likes, and writes a reply ([Model](reference/runs.md#model)). decree writes the request, runs the router as a child run, and validates the reply. Prompts, models, retries and budgets all live in the router, so changing how a decision is made is a machine and script edit, never a decree release.

## Typed and untyped

Routers come in two kinds:

| Kind | How the reply is constrained | What its confidence means | How it can fail | Cost | Use it for |
| --- | --- | --- | --- | --- | --- |
| **Typed**: a classifier ([GLiNER2.5-Decide](#fastino-gliner25-decide-run-locally)), constrained decoding ([Ollama's `format`](#ollama-with-format), vLLM's guided choice, SGLang's `select`), [TypeSafe Jev](#typesafe-jev) | By the model: it cannot answer outside the options. A classifier picks one of the labels it is given; a decoder is held to the request's `reply_schema`; Jev picks among its criteria. | A classifier's score for its pick, or Jev's figure from its probability distribution: measured. A constrained language model's number is still its own estimate. | Its service is down or times out: the router run fails, and the event is `error`. | A classifier on CPU: milliseconds, no fee. A local model: seconds of GPU. Jev: per call. | **Routing**: the cheap, frequent, bounded decisions, such as which model does the work or which path a message takes. |
| **Untyped**: a chat model or coding agent ([`claude -p`, Copilot, OpenCode](#claude-copilot-and-opencode-written-by-decree-init)) | Not at all: the script asks for JSON in the prompt, finds it in free text and checks it. | Self-reported: the least reliable kind. | No JSON, or a pick that is not an option: the script fails, and the event is `error` once its attempts run out. | Tokens and seconds per call, and a subscription or a bill. | **The work** itself, and judgments that need reasoning over a lot of context. |

Use a typed router to decide, and an untyped model to do. A routing decision runs on every message, so it should be cheap, and its answer must be one of the options, so it should not be able to say anything else: a classifier such as GLiNER2.5-Decide is the one to reach for, and its score is a real one to set `min_confidence` against. Claude, Copilot and OpenCode are for writing the code and for questions that need the whole message and the codebase to answer; asked to route, they cost more, can reply off the options, and report a confidence that means little. [`examples/route-by-complexity/`](../examples/route-by-complexity/README.md) puts both together: GLiNER decides whether a change is small enough for a local model, or needs Claude.

Running a model server is outside decree: these routers only talk to one (see [`docs/services.md`](services.md) for running services next to decree).

## The contract

A router is any machine. A `model` invoke picks one with `router: <machine>`; without it, the machine named `router` is used, which `decree init` writes (`decree check` fails V16 if a `model` names no router and there is no `machines/router.yml`):

```yaml
# machines/sort_document.yml
local_model:
  invoke:
    model:
      question: Which kind of document is this?
      router: gliner_router
      min_confidence: 0.9
      output: read_text
  transitions:
    invoice:
      target: file_invoice
      description: A bill asking for payment, with an amount due.
    receipt:
      target: file_receipt
      description: Proof of a payment already made.
    other:
      target: file_other
      description: Any other paperwork.
    unsure: big_model
```

Its scripts get two extra variables ([Environment](reference/scripts.md#environment)):

| Variable | Value |
| --- | --- |
| `DECREE_REQUEST` | Absolute path of `request.json` in the router run's folder. |
| `DECREE_REPLY` | Absolute path where the reply must be written, `reply.json` in the same folder. |

**Request** (`request.json`, written by decree before the router starts):

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
  "reply_schema": {"$schema": "https://json-schema.org/draft/2020-12/schema", "type": "object", "properties": {"event": {"enum": ["retry", "split"]}, "…": "…"}, "required": ["event"], "additionalProperties": false}
}
```

`options` are the state's transitions except `unsure` and `error`, in name order. `min_confidence` is present only when the invoke sets it; decree applies it, so a router only reports. `input` is what the invoke's `output` state's script printed, and empty when it names no `output`: a script decides what the model sees, which is also how secrets stay out of a prompt. `reply_schema` is a JSON Schema (draft 2020-12) for the reply below, over this request's options ([Model](reference/runs.md#model)): a typed router hands it to a constrained decoder unchanged.

**Reply** (`reply.json`, written by the router):

```json
{"event": "retry", "reason": "One test fails on an off-by-one.", "confidence": 0.86, "probabilities": {"retry": 0.86, "split": 0.14}}
```

Only `event` is required. When present, `reason` is a string, `confidence` a number from 0 to 1, and `probabilities` a map of option to number.

**Validation.** decree reads the reply when the router run reaches a final state:

- The router run ended in `failed`, `reply.json` is missing or malformed, or its `event` is not exactly one of the options: the state's event is `error`, and the `decision` event says why in `router_error`. decree never fuzzy-matches an option.
- With `min_confidence` set, a missing or lower `confidence` gives `unsure` instead of the pick.
- Otherwise the pick is the event.

The `decision` event records the router, the router's run id (`child_run`), the pick (even when the event is `unsure`), the reason, the confidence and the probabilities.

## Confidence is calibrated per router

`min_confidence` compares against the router's own number, and routers produce very different numbers:

| Router | Where its confidence comes from |
| --- | --- |
| Chat models (`router`, as `decree init` writes it) | The model's own estimate in its reply. The least reliable: chat models tend to report high numbers whatever they pick. |
| TypeSafe Jev | Computed from the shape of its probability distribution over the options. |
| GLiNER2.5-Decide | The classifier's score for the label it picks (`include_confidence=True`). |
| A language model constrained by `format` | Still the model's own estimate: the decoder constrains the shape of the reply, not how honest its number is. |

So a threshold only means something for the router it was set with. When a `model` invoke switches to another router, or `machines/router.yml` changes, revisit every `min_confidence` that uses it. Every `decision` event records `router`, `pick`, `confidence` and the outcome that followed, so you can check a threshold against what actually happened (for example in Grafana, [Observability](reference/observability.md)) before trusting it.

## Typed routers

### Fastino GLiNER2.5-Decide, run locally

[GLiNER2.5-Decide](https://huggingface.co/fastino/GLiNER2.5-Decide-1B) is a typed router: a 1B classifier built for operational decisions, which picks one of the labels it is given and reports its score for that label. It runs on CPU (Apache-2.0). Loading it takes far longer than one decision, so a small long-running server, [`examples/route-by-complexity/gliner/decide_server.py`](../examples/route-by-complexity/gliner/decide_server.py), loads it once and answers `POST /classify` on `127.0.0.1:8090`: `{instructions, labels: {name: description}, text}` in, `{event, confidence}` out. `GET /health` answers `{"ok": true}`; the server only listens once the model is loaded, so any answer means ready. Quick start, from `examples/route-by-complexity/` (Python 3.10 or newer):

```sh
pip install 'gliner2[local]'           # or skip this and start it with: uv run --with 'gliner2[local]' python gliner/decide_server.py
python gliner/decide_server.py         # the first start downloads the model, about 4.8 GB
curl -s 127.0.0.1:8090/classify -d '{"instructions": "How much reasoning does this change need?", "labels": {"small": "A typo or a config value", "large": "Design across several files"}, "text": "Fix a typo in README.md"}'
```

To keep it running, use the `decide` systemd user unit in [`docs/services.md`](services.md#systemd-user-units-only-one-of-these-at-a-time), or a tmux session as in [`examples/tmux-services/`](../examples/tmux-services/README.md). The `gliner2` package's plain install is only its API client; `[local]` adds local inference ([gliner2 README](https://github.com/fastino-ai/GLiNER2)).

`machines/gliner_router.yml` (in [`examples/sort-documents/`](../examples/sort-documents/README.md), [`examples/route-by-complexity/`](../examples/route-by-complexity/README.md) and [`examples/tmux-services/`](../examples/tmux-services/README.md)):

```yaml
name: gliner_router
description: Ask the local GLiNER2.5-Decide classifier to pick one of the options in the request.
initial: ask
states:
  ask:                             # posts the question, options and input; writes $DECREE_REPLY
    invoke: ask_gliner
    transitions: { done: done }
  done:   { final: true }
  failed: { final: true }
```

`scripts/gliner_router/ask_gliner.sh` sends the question, the options with their descriptions as labels, and `input` and `message_body` as the text, and writes the server's answer as the reply:

```bash
url="${GLINER_URL:-http://127.0.0.1:8090/classify}"
body=$(jq '{instructions: .question,
            labels: (.options | map({(.event): .description}) | add),
            text: ([.input, .message_body] | map(select(. != "")) | join("\n\n"))}' "$DECREE_REQUEST")
reply=$(curl -fsS --max-time 30 -H 'content-type: application/json' -d "$body" "$url")
printf '%s\n' "$reply" > "$DECREE_REPLY"
jq -r '"picked \(.event)"' "$DECREE_REPLY"   # for the log only, as in ask_claude
```

`classify_text` takes the labels with their descriptions, and no separate instructions, so the server puts the question in front of the text. The pages document a score for the picked label only, so the reply has no `probabilities`. If the server is down, `curl` fails, the router run ends in `failed`, and the state's event is `error`: give the state an `error` transition (to a bigger model, or to a person) if a missing classifier should not fail the run, as `develop_by_size` does.

### Ollama with `format`

A language model served by [Ollama](https://docs.ollama.com/capabilities/structured-outputs) is a typed router when its call passes the request's `reply_schema` as `format`: Ollama constrains the model's output to the JSON Schema it is given, and the schema's `event` enum holds the pick to the options. Ollama's docs suggest also putting the schema in the prompt, and a temperature of 0, and say the same works through its OpenAI-compatible API's `response_format`. vLLM's guided decoding takes a JSON Schema too; SGLang's `select` picks among the option names directly. The confidence is still the model's own estimate.

`machines/ollama_router.yml` is `gliner_router` with `invoke: ask_ollama`; `scripts/ollama_router/ask_ollama.sh`:

```bash
#!/usr/bin/env bash
# ollama_router's only script: one chat call whose output Ollama holds to reply_schema.
set -euo pipefail
model="${OLLAMA_MODEL:-qwen3-coder:30b}"
body=$(jq --arg model "$model" '{
  model: $model, stream: false, format: .reply_schema, options: {temperature: 0},
  messages: [{role: "user", content: (
    "Question: \(.question)\n\nOptions:\n" +
    (.options | map("- \(.event): \(.description)") | join("\n")) +
    "\n\nOutput this decision reads:\n\(.input)\n\nTask message:\n\(.message_body)\n\n" +
    "Reply with JSON that matches this schema:\n" + (.reply_schema | tojson))}]
}' "$DECREE_REQUEST")
echo "$body"
echo "--- reply"
answer=$(curl -fsS --max-time 120 http://127.0.0.1:11434/api/chat -d "$body")
echo "$answer"
jq '.message.content | fromjson' <<<"$answer" > "$DECREE_REPLY"
jq -r '"picked \(.event)"' "$DECREE_REPLY"
```

The reply is in the response's `message.content`. Ollama's docs do not list which JSON Schema keywords `format` supports, so check that your version honours `enum` before relying on it; decree validates the reply either way.

### TypeSafe Jev

[Jev](https://docs.typesafe.ai/primitives/choice.md) is a typed router: a hosted decision model that picks among the criteria it is given. A request maps onto one Choice question:

| Request | Jev |
| --- | --- |
| `question` | the question's `instructions` |
| `options` (event → description) | the question's `criteria` |
| `input` and `message_body` (with the workflow and step) | `state` |
| answer `choice` | reply `event` |
| answer `probabilities` | reply `probabilities` |
| answer `confidence` | reply `confidence` |

`machines/jev_router.yml`:

```yaml
name: jev_router
description: Ask TypeSafe Jev to choose one of the options in the request.
initial: ask
states:
  ask:
    invoke:                        # network errors: try once more
      script: { name: ask_jev, max_attempts: 2 }
    transitions: { done: done }
  done:   { final: true }
  failed: { final: true }
```

`scripts/jev_router/ask_jev.sh`:

```bash
#!/usr/bin/env bash
# jev_router's only script: one Choice question per decision.
# Needs TYPESAFE_API_KEY in decree's environment.
set -euo pipefail
body=$(jq '{
  model: "jev-latest",
  state: ({workflow: "\(.machine): \(.machine_description)",
           step: "\(.state): \(.state_description)",
           input: .input, message: .message_body, history: .history} | tojson),
  questions: {decision: {type: "choice",
                         instructions: .question,
                         criteria: (.options | map({(.event): .description}) | add)}}
}' "$DECREE_REQUEST")
echo "$body"
echo "--- reply"
answer=$(curl -fsS --max-time 60 https://api.typesafe.ai/v1/systemone \
  -H "authorization: Bearer $TYPESAFE_API_KEY" -H 'content-type: application/json' -d "$body")
echo "$answer"
jq '.answers.decision | {event: .choice, confidence, probabilities}' <<<"$answer" > "$DECREE_REPLY"
```

Check the authorization header against Jev's [API reference](https://docs.typesafe.ai/api.md) for your account. Jev picks among the criteria it was given, so a reply that is not an option should not happen; decree checks anyway. TypeSafe suggests a confidence floor around 0.6 as a starting point; calibrate it as described above.

### OpenAI Decisions API (pending)

OpenAI's Decisions API would be a typed router: it takes context and questions with fixed answer lists and returns one answer per question, which is the same shape as a `model` request. As of 2026-10-02 it is in limited preview, and its schema, endpoint and pricing are not published. A router for it will be the same two-file pattern as `jev_router`: `question` as the question, the options as its answers, `input` and `message_body` as context, and the answer back as `event`, with a confidence if the API reports one. This section will get its machine and script once the schema is public.

## Untyped routers

### Claude, Copilot and OpenCode (written by `decree init`)

The default `router` is untyped: it asks a chat model or coding agent for free text and looks for the reply in it. `decree init --ai claude` writes `machines/router.yml` and its script `scripts/router/ask_claude.sh`:

```yaml
# yaml-language-server: $schema=../schema/v1/machine.schema.json
# Graph: ../graph/router.md
name: router
description: Ask Claude to pick one of the options in the request.
initial: ask
states:
  ask:                             # renders the prompt from $DECREE_REQUEST, runs claude -p, writes $DECREE_REPLY
    invoke:                        # a reply that is not one of the options fails the script; it runs once more
      script: { name: ask_claude, max_attempts: 2 }
    transitions: { done: done }
  done:   { final: true }
  failed: { final: true }
```

`ask_claude` renders the prompt in [The default router](reference/runs.md#the-default-router) with `jq`, sends it to `claude -p`, takes the last JSON object from the reply (the last fenced block if there is one, which may span lines; else the last line holding an object), and exits non-zero unless its `event` is one of the options: a reply with no JSON, or with a pick that is not an option, fails the script, and once its attempts run out the state's event is `error`. It does not use `reply_schema`: the reply is free text, checked after the fact. It needs `jq` on `PATH`.

`--ai copilot` and `--ai opencode` write the same `router` machine with `ask_copilot` or `ask_opencode` (`scripts/router/ask_<ai>.sh`). Only the script's name and the line that calls the CLI differ:

| `--ai` | Script | The call |
| --- | --- | --- |
| `claude` | `ask_claude` | `printf '%s' "$prompt" \| claude -p` |
| `copilot` | `ask_copilot` | `copilot -p "$prompt"` |
| `opencode` | `ask_opencode` | `opencode run "$prompt"` |

## Cheap model first, stronger model when unsure

An escalating router is as typed as the models it asks: typed if both are, untyped if either is not. A router can itself escalate: ask a cheap model, and only ask a stronger one when the first is not sure. Because the router is a machine, the escalation is visible in its graph and its run's events:

```yaml
name: escalating_router
description: Ask a small model first, and a large one only when the small one is unsure.
initial: cheap
states:
  cheap:                           # writes $DECREE_REPLY; names the event sure when its confidence is at least 0.85
    invoke: ask_cheap
    transitions: { sure: done, done: strong }
  strong:                          # overwrites $DECREE_REPLY
    invoke: ask_strong
    transitions: { done: done }
  done:   { final: true }
  failed: { final: true }
```

`ask_cheap` is `ask_gliner` (or `ask_claude` with a small model) plus one line at the end:

```bash
jq -e '(.confidence // 0) >= 0.85' "$DECREE_REPLY" > /dev/null && echo sure > "$DECREE_EVENT_FILE"
exit 0
```

The `0.85` belongs to the cheap model, and the `min_confidence` on the deciding state then applies to whichever model answered last; calibrate both. The same ladder can also be built in the deciding machine instead, with two `model` states and an `unsure` transition between them, as `sort_document` in [`examples/sort-documents/`](../examples/sort-documents/README.md) does; that puts the escalation in the main machine's graph rather than inside the router.
