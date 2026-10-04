# Router machines

A `model` state asks a **router**: an ordinary machine that reads a request, asks a model however it likes, and writes a reply ([Model](reference/runs.md#model)). decree writes the request, runs the router as a child run, and validates the reply. Prompts, models, retries and budgets all live in the router, so changing how a decision is made is a machine and script edit, never a decree release.

This page shows router machines for several backends. Running a model server is outside decree: these routers only talk to one (see `docs/services.md` for running services next to decree).

## The contract

A router is any machine. A `model` invoke picks one with `router: <machine>`; without it, the machine named `router` is used, which `decree init` writes (`decree check` fails V16 if a `model` names no router and there is no `machines/router.yml`):

```yaml
# machines/sort_document.yml
local_model:
  invoke:
    model:
      question: Which kind of document is this?
      router: local_router
      min_confidence: 0.9
      output: read_text
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
  "history": ["precheck: done", "implement: done", "verify: fail", "rounds_left: true"]
}
```

`options` are the state's transitions except `unsure` and `error`, in name order. `min_confidence` is present only when the invoke sets it; decree applies it, so a router only reports. `input` is what the invoke's `output` state's script printed, and empty when it names no `output`: a script decides what the model sees, which is also how secrets stay out of a prompt.

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
| GLiNER2.5-Decide | The classifier's score for the label it picks. |

So a threshold only means something for the router it was set with. When a `model` invoke switches to another router, or `machines/router.yml` changes, revisit every `min_confidence` that uses it. Every `decision` event records `router`, `pick`, `confidence` and the outcome that followed, so you can check a threshold against what actually happened (for example in Grafana, [Observability](reference/observability.md)) before trusting it.

## Claude, Copilot and OpenCode (written by `decree init`)

`decree init --ai claude` writes `machines/router.yml` and its script `scripts/router/ask_claude.sh`:

```yaml
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

`ask_claude` renders the prompt in [The default router](reference/runs.md#the-default-router) with `jq`, sends it to `claude -p`, takes the last JSON object from the reply (the last fenced block if there is one, which may span lines; else the last line holding an object), and exits non-zero unless its `event` is one of the options. It needs `jq` on `PATH`.

`--ai copilot` and `--ai opencode` write the same `router` machine with `ask_copilot` or `ask_opencode` (`scripts/router/ask_<ai>.sh`). Only the script's name and the line that calls the CLI differ:

| `--ai` | Script | The call |
| --- | --- | --- |
| `claude` | `ask_claude` | `printf '%s' "$prompt" \| claude -p` |
| `copilot` | `ask_copilot` | `copilot -p "$prompt"` |
| `opencode` | `ask_opencode` | `opencode run "$prompt"` |

## TypeSafe Jev

[Jev](https://docs.typesafe.ai/primitives/choice.md) is a hosted decision model. A request maps onto one Choice question:

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

## Fastino GLiNER2.5-Decide, run locally

[GLiNER2.5-Decide](https://huggingface.co/fastino/GLiNER2.5-Decide-1B) is a 1B classifier built for operational decisions. It runs on CPU (Apache-2.0, `pip install gliner2`). Loading it takes far longer than one decision, so do not start Python per decision: run a small long-running server that loads the model once, and let the router talk to it. The `local_router` machine in `mock/` does exactly this.

The server (yours, not decree's; run it as a service, `docs/services.md`), `decide_server.py`:

```python
# Loads GLiNER2.5-Decide once and classifies over HTTP on 127.0.0.1:8090.
# POST /classify {"instructions": str, "labels": {name: description}, "text": str}
#   -> {"<picked label>": <confidence>}
import json
from http.server import BaseHTTPRequestHandler, HTTPServer
from gliner2 import AutoExtractor

model = AutoExtractor.from_pretrained("fastino/GLiNER2.5-Decide-1B")

class Classify(BaseHTTPRequestHandler):
    def do_POST(self):
        req = json.loads(self.rfile.read(int(self.headers["content-length"])))
        # Described labels, so the classifier reads what each option means. The question
        # goes in front of the text: classify_text takes no separate instructions.
        result = model.classify_text(
            f"{req['instructions']}\n\n{req['text']}",
            {"decision": {"labels": req["labels"]}},
            include_confidence=True,
        )["decision"]
        body = json.dumps({result["label"]: result["confidence"]}).encode()
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.end_headers()
        self.wfile.write(body)

HTTPServer(("127.0.0.1", 8090), Classify).serve_forever()
```

`machines/local_router.yml` (as in `mock/`):

```yaml
name: local_router
description: Ask the local classifier to score the options in the request.
initial: ask
states:
  ask:
    invoke: ask_local
    transitions: { done: done }
  done:   { final: true }
  failed: { final: true }
```

`scripts/local_router/ask_local.sh` sends the question, the options with their descriptions and the input, and writes the top label as the reply with its score:

```bash
#!/usr/bin/env bash
set -euo pipefail
url="${DECIDE_URL:-http://127.0.0.1:8090/classify}"
body=$(jq '{instructions: .question,
            labels: (.options | map({(.event): .description}) | add),
            text: .input}' "$DECREE_REQUEST")
echo "$body"
echo "--- reply"
scores=$(curl -fsS --max-time 30 -H 'content-type: application/json' -d "$body" "$url")
echo "$scores"
jq '{probabilities: .} + (to_entries | max_by(.value) | {event: .key, confidence: .value})' \
  <<<"$scores" > "$DECREE_REPLY"
```

`include_confidence=True` reports the score of the picked label. If the server returns a score for every label, the script passes them all on as `probabilities`. If the server is down, `curl` fails, the router run ends in `failed`, and the state's event is `error`: give the state an `error` transition (to a bigger model or to a person) if a missing classifier should not fail the run.

## OpenAI Decisions API (pending)

OpenAI's Decisions API takes context and questions with fixed answer lists and returns one answer per question, which is the same shape as a `model` request. As of 2026-10-02 it is in limited preview, and its schema, endpoint and pricing are not published. A router for it will be the same two-file pattern as `jev_router`: `question` as the question, the options as its answers, `input` and `message_body` as context, and the answer back as `event`, with a confidence if the API reports one. This section will get its machine and script once the schema is public.

## A self-hosted LLM

A local LLM behind an OpenAI-compatible endpoint (SGLang, vLLM, Ollama) is the `router` machine with a different call. Constrain the output to the options where the server supports it (SGLang's `select`, vLLM's guided choice), so the reply is always one of them; a self-reported confidence is as weak as any chat model's.

## Cheap model first, stronger model when unsure

A router can itself escalate: ask a cheap model, and only ask a stronger one when the first is not sure. Because the router is a machine, the escalation is visible in its graph and its run's events:

```yaml
name: escalating_router
description: Ask a small model first, and a large one only when the small one is unsure.
initial: cheap
states:
  cheap:                           # writes $DECREE_REPLY; prints {"event":"sure"} when its confidence is at least 0.85
    invoke: ask_cheap
    transitions: { sure: done, done: strong }
  strong:                          # overwrites $DECREE_REPLY
    invoke: ask_strong
    transitions: { done: done }
  done:   { final: true }
  failed: { final: true }
```

`ask_cheap` is `ask_local` (or `ask_claude` with a small model) plus one line at the end:

```bash
jq -e '(.confidence // 0) >= 0.85' "$DECREE_REPLY" > /dev/null && echo '{"event":"sure"}'
exit 0
```

The `0.85` belongs to the cheap model, and the `min_confidence` on the deciding state then applies to whichever model answered last; calibrate both. The same ladder can also be built in the deciding machine instead, with two `model` states and an `unsure` transition between them, as `sort_document` in `mock/` does; that puts the escalation in the main machine's graph rather than inside the router.
