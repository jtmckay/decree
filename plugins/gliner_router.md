---
name: gliner_router
machine:
  description: Ask the local GLiNER2.5-Decide classifier to pick one of the options in the request.
  concurrency: 4
  initial: ask
  states:
    ask:
      invoke: ask_gliner
      transitions:
        done: done
    done:
      final: true
    failed:
      final: true
scripts:
  ask_gliner.sh: |
    #!/usr/bin/env bash
    # gliner_router's only script: one classification per decision. Sends the
    # request's question, its options with their descriptions, and its input and
    # message body to the GLiNER2.5-Decide server, and writes the server's answer,
    # {"event": ..., "confidence": ...}, as the reply. decree applies
    # min_confidence; this script only reports. If the server is down, curl fails,
    # the router run ends in `failed`, and the deciding state's event is `error`.
    # The server: examples/route-by-complexity/gliner/decide_server.py.
    set -euo pipefail
    url="${GLINER_URL:-http://127.0.0.1:8090/classify}"
    body=$(jq '{instructions: .question,
                labels: (.options | map({(.event): .description}) | add),
                text: ([.input, .message_body] | map(select(. != "")) | join("\n\n"))}' "$DECREE_REQUEST")
    echo "$body"
    echo "--- reply"
    reply=$(curl -fsS --max-time 30 -H 'content-type: application/json' -d "$body" "$url")
    echo "$reply"
    printf '%s\n' "$reply" > "$DECREE_REPLY"
    # for the log only: decree takes the event from the reply
    jq -r '"picked \(.event)"' "$DECREE_REPLY"
---
# gliner_router

A typed router: Fastino's GLiNER2.5-Decide, a classifier, picks one of the option labels it is given, so its answer is always an option and its confidence is the classifier's score. Once loaded it costs milliseconds of CPU per decision. Use it when the answer shows in the wording and there are a few options; it is poor at judging.

## What it needs

- `curl` and `jq`.
- The classifier server, `decide_server.py`, answering `POST /classify` on `127.0.0.1:8090` (`GLINER_URL`). It is in decree's repository, `examples/route-by-complexity/gliner/decide_server.py`, with how to install and run it (Python 3.10 or newer, `pip install 'gliner2[local,train]'`).

## How to use it

Point a model state's `router:` at it:

```yaml
    invoke:
      model:
        question: How much reasoning does this change need?
        router: gliner_router
        min_confidence: 0.7
```
