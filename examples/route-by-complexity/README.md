# Route by complexity: a classifier picks the model

This example sends each change to the cheapest model that can do it. A local classifier, Fastino's [GLiNER2.5-Decide](https://huggingface.co/fastino/GLiNER2.5-Decide-1B), reads the change and its size, and decides whether a local model served by Ollama can do it, or it needs Claude.

- The classifier is **typed**: it can only answer with one of the labels it is given, and its confidence is its own score. Once loaded, it costs milliseconds of CPU and no money per decision, so it runs on every message.
- Claude is **untyped**: it answers in free text, and costs money. It runs only when the classifier says the change needs it, when the classifier is not sure, or when the local model's work fails the tests.

[`docs/routers.md`](../../docs/routers.md) explains the two kinds of router.

## The machine

[`machines/develop_by_size.yml`](.decree/machines/develop_by_size.yml) ([graph](.decree/graph/develop_by_size.md)):

```mermaid
stateDiagram-v2
    [*] --> describe
    claude_only --> done: done
    claude_only --> failed: error (implicit)
    claude_only --> failed: stop
    describe --> size_up: done
    describe --> failed: error (implicit)
    local_first --> done: done
    local_first --> failed: error (implicit)
    local_first --> failed: stop
    size_up --> claude_only: error
    size_up --> claude_only: large (model: gliner_router)
    size_up --> local_first: small (model: gliner_router)
    size_up --> claude_only: unsure (model: gliner_router)
    done --> [*]
    failed --> [*]
    note right of claude_only
        attempts: claude → claude
    end note
    note right of local_first
        attempts: local → local → claude
    end note
    note right of size_up
        model: gliner_router, min_confidence 0.7
    end note
```

1. `describe` prints what the classifier reads: the message's title, its acceptance criteria, and the files it names that exist, with their line counts, so the classifier sees a size and not only prose.
2. `size_up` asks [`gliner_router`](.decree/machines/gliner_router.yml) "How much reasoning does this change need?", with two options: `small` ("A local, mechanical change: a typo, a rename, a config value, one small function with a clear spec.") and `large` ("Design, several files, unclear requirements, concurrency or security."). Below `min_confidence: 0.7` the event is `unsure`, and `unsure` goes to Claude: when in doubt, use the stronger model. So does `error`, when the classifier is not running.
3. `local_first` and `claude_only` run one script, [`implement`](.decree/scripts/develop_by_size/implement.sh), once per entry of their attempt list until an attempt succeeds. The entry, in `$DECREE_ATTEMPT_VALUE`, names the tool: `local` runs `opencode run --model ollama/<model>`, and `claude` runs `claude -p`, logging each step in the run's `progress.md` and writing `STOP` instead of guessing, as `rust_develop` does. A `STOP` from any attempt is the `stop` event, which ends the run in `failed` until a person answers it.
4. Each attempt then runs `$TEST_CMD` (`cargo test` by default), and fails if the tests fail. `local_first` tries `local`, `local`, then `claude`; `claude_only` tries `claude` twice. If every attempt fails, the event is `error`, and the run ends in `failed`.

The tests run inside the attempt because an attempt succeeds when the change passes the tests, not when the model stops talking. So a local model's change that fails them moves to the next attempt, in the same state, with no state to count the tries.

## Running it

These commands only read the project:

```bash
cd examples/route-by-complexity
decree check                             # every machine and message is valid
decree graph                             # rewrites .decree/graph/ with no change
```

To use it, copy `.decree/` into a project and queue a change, such as [`migrations/01-raise-upload-limit.md`](.decree/migrations/01-raise-upload-limit.md). It needs three things running: the classifier, Ollama with a coding model, and OpenCode and Claude Code on `PATH`.

### The classifier, GLiNER2.5-Decide

[`gliner/decide_server.py`](gliner/decide_server.py) loads the model once and answers `POST /classify` on `127.0.0.1:8090`, so no decision starts Python. It needs Python 3.10 or newer:

```sh
pip install 'gliner2[local]'           # or skip this and start it with: uv run --with 'gliner2[local]' python gliner/decide_server.py
python gliner/decide_server.py         # the first start downloads the model, about 4.8 GB
curl -s 127.0.0.1:8090/classify -d '{"instructions": "How much reasoning does this change need?", "labels": {"small": "A typo or a config value", "large": "Design across several files"}, "text": "Fix a typo in README.md"}'
```

The `curl` prints the reply as `gliner_router` writes it, `{"event": "small", "confidence": ...}`. To keep the server running, use the `decide` systemd user unit in [`docs/services.md`](../../docs/services.md).

### The local model, through OpenCode

`implement` names the local model in one variable at its top, `OLLAMA_MODEL`, which defaults to `qwen3-coder:30b` (19 GB). Pull it, and tell OpenCode about Ollama in the project's `opencode.json` ([OpenCode's Ollama provider](https://opencode.ai/docs/providers/)):

```sh
ollama pull qwen3-coder:30b
```

```json
{
  "$schema": "https://opencode.ai/config.json",
  "provider": {
    "ollama": {
      "npm": "@ai-sdk/openai-compatible",
      "name": "Ollama (local)",
      "options": { "baseURL": "http://localhost:11434/v1" },
      "models": { "qwen3-coder:30b": { "name": "Qwen3 Coder 30B" } }
    }
  }
}
```

OpenCode's docs suggest raising Ollama's `num_ctx` (16k to 32k) if tool calls do not work.

## Tuning `min_confidence`

0.7 is a starting point, and it belongs to this classifier ([Confidence is calibrated per router](../../docs/routers.md#confidence-is-calibrated-per-router)). Every `size_up` records a `decision` event with the pick, the confidence and the event that followed, and every failed attempt is a `transition` event with `source: "attempt"` and the `attempt_value` of the attempt it starts. Ship them to Loki as in [`observability`](../observability/README.md), and compare in Grafana:

```logql
# what the classifier picked, and how sure it was
{job="decree", type="decision", machine="develop_by_size"} | json | state="size_up" | line_format "{{.run_id}} {{.pick}} {{.confidence}} {{.event}}"

# local attempts that failed the tests: each starts the next attempt, and "claude" means both failed
{job="decree", type="transition", machine="develop_by_size"} | json | source="attempt" and from="local_first" | line_format "{{.run_id}} next: {{.attempt_value}}"
```

If `small` picks above the threshold often fail the tests, raise it; if `unsure` changes that Claude then did were mostly small, lower it.

## The files

```text
examples/route-by-complexity/
  gliner/decide_server.py              the classifier server (the one copy of it in the repository)
  .decree/
    machines/
      develop_by_size.yml              size up, then implement and test: locally first, or with Claude only
      gliner_router.yml                a typed router: asks the classifier server (the same file as in sort-documents)
    scripts/
      develop_by_size/describe.sh      prints the title, acceptance criteria and named files with line counts
      develop_by_size/implement.sh     one attempt: opencode with an Ollama model (local) or claude -p (claude), then $TEST_CMD
      gliner_router/ask_gliner.sh      posts the request to the classifier server, writes the reply
    migrations/01-raise-upload-limit.md   a small change to try it with
    graph/  schema/                    written by `decree graph` and `decree schema`
```
