---
machine: rust_develop
---
# 74: Typed and untyped routers, and GLiNER in a few lines

## Overview

A `model` decision is answered by a router machine, and routers come in two kinds that the docs do not tell apart today:

- **Typed.** The model cannot answer outside the options.
  - A classifier such as Fastino's GLiNER2.5-Decide scores each option label, so its answer is always one of them, and its confidence is a real score.
  - Constrained decoding (Ollama's `format` with a JSON Schema, vLLM's guided choice, SGLang's `select`) forces a language model's output into the reply's shape.
  - TypeSafe Jev picks among the criteria it is given.
- **Untyped.** A chat model or coding agent answers in free text (Claude Code's `claude -p`, Copilot, OpenCode).
  - The router script has to find a JSON object in the text and check it, and it can fail: no JSON, or a pick that is not an option, which ends in `error`.
  - Its confidence is self-reported, the least reliable kind.

Decided (by the user):

- Use a **typed** router for routing decisions: the cheap, frequent, bounded ones. GLiNER is the one to reach for.
- Use an **untyped** model for the work itself, and for judgments that need reasoning over a lot of context.
- The flagship example routes **by complexity**. GLiNER, running locally, decides whether a change is small enough for a local model served by Ollama, or needs Claude.

## Requirements

Read `docs/routers.md`, `docs/services.md`, `docs/reference/runs.md` (Model, The default router, `request.json`), `examples/sort-documents/` (after migration 73) and `tests/README.md` first.

1. **`docs/routers.md`** opens with the two kinds:
   - a table with columns for the kind, how the reply is constrained, what confidence means, how it can fail, its cost, and when to use it;
   - one paragraph of guidance: typed routers for routing, untyped models for the work.

   Then one section per router, grouped under **Typed** (GLiNER, Ollama with `format`, Jev) and **Untyped** (the default `router` that `decree init` writes for Claude, Copilot and OpenCode). Each section says which kind it is in its first sentence.
2. **`reply_schema` in `request.json`.** decree adds a field `reply_schema` to every `request.json`: a JSON Schema (draft 2020-12) for `reply.json`. Its fields:
   - `event`: an enum of the options;
   - `confidence`: a number from 0 to 1;
   - `reason`: a string;
   - `probabilities`: an object of numbers, keyed by option.

   `event` is required and `additionalProperties` is false. Typed routers pass it to a constrained decoder unchanged (Ollama's `format`, OpenAI-style `response_format`), which is what makes them typed. Document the field in `docs/reference/runs.md` with the other request fields. Recorded `request.json` files in `examples/` gain the field, and the replay tests compare it.
3. **GLiNER in a few lines.** One copy of a small server, `examples/route-by-complexity/gliner/decide_server.py`:
   - Python standard library `http.server` plus `gliner2`, no web framework;
   - it loads `fastino/GLiNER2.5-Decide-1B` once with `gliner2.AutoExtractor.from_pretrained`;
   - it serves `POST /classify` on `127.0.0.1:8090`: `{instructions, labels: {name: description}, text}` in, `{event, confidence, probabilities?}` out.

   Verify the `gliner2` API (`classify_text`, how labels with descriptions are passed, `include_confidence`, whether per-label scores are available) against the model card at https://huggingface.co/fastino/GLiNER2.5-Decide-1B and the `gliner2` README before writing it. Fetching those pages is allowed. Write only what they confirm, and say in a comment which parts they confirmed.

   The quick start in `docs/routers.md` and in the example's README is at most five commands: install (`pip install gliner2`, or `uv run --with gliner2`), start the server (the first start downloads the model; say its size), one `curl` to try it, and a pointer to the systemd user unit in `docs/services.md` for running it as a service. Replace the server code currently inlined in `docs/routers.md` with a link to the file.
4. **`gliner_router` machine.** It replaces `local_router` in `examples/sort-documents/` and is used by `examples/route-by-complexity/`, a copy in each. It has one state and one script, `ask_gliner.sh`, which:
   - sends `question`, the options with their descriptions, and `input` plus `message_body` to the server;
   - writes the reply;
   - ends its stdout with a plain line, as the default router does.

   Rename `local_router` everywhere in `examples/sort-documents/`, including its recorded run, and keep the replay test green.
5. **`examples/route-by-complexity/`**, a project that passes `decree check` and `decree graph`. Its machine `develop_by_size`:
   - `describe` (script): prints what the classifier should read. That is the message's title and acceptance criteria, the files the message names that exist, and their line counts, so GLiNER sees a size, not just prose.
   - `size_up`: `model` with `router: gliner_router`, the question "How much reasoning does this change need?", `min_confidence: 0.7` and `output: describe`. Two options:
     - `small`: "A local, mechanical change: a typo, a rename, a config value, one small function with a clear spec." → `implement_local`.
     - `large`: "Design, several files, unclear requirements, concurrency or security." → `implement_claude`.

     `unsure` → `implement_claude`: when in doubt, use the stronger model.
   - `implement_local`: a script that implements the message with a local model served by Ollama, through OpenCode (`opencode run` with an Ollama model). Verify how OpenCode selects an Ollama model against its docs; fetching them is allowed. Put the model name in one variable at the top, defaulting to a current coding model on Ollama, and document what to install (`ollama pull <model>`).
   - `implement_claude`: implements with `claude -p`, like `rust_develop`'s implement, with the same STOP and progress rules.
   - `verify`: runs the project's tests (a `TEST_CMD` variable, default `cargo test`).
     - `done` → `done`.
     - `error` → `tried_claude`, a `check` on `{ visits: implement_claude, less_than: 1 }`: `true` → `implement_claude` (a failed local attempt escalates once to Claude), `false` → `failed`.

   Its README explains the idea in a few lines:
   - the classifier is typed and costs milliseconds on CPU, so it runs on every message;
   - Claude is untyped and costs money, so it runs only when the classifier says the change needs it, or when local work fails verification.

   Show the graph, and how to tune `min_confidence` from the `decision` events in Grafana (`examples/observability/`).
6. **Tests** (no network, no model). A stub `ask_gliner.sh` writes a chosen reply, and stub `opencode`, `claude` and test commands go on `PATH`. Run `develop_by_size` through the binary for each path, and assert the path taken:
   - `small` → local → `done`;
   - `large` → Claude → `done`;
   - `unsure` → Claude;
   - `small` → local → verify fails → Claude → `done`;
   - `small` → local fails, Claude fails → `failed`.

   Also test that every `request.json` decree writes has a `reply_schema` whose `event` enum equals the options, and that it validates the recorded `reply.json` files (the `jsonschema` dev-dependency is already there). Byte-compile the server with `python3 -m py_compile`, skipping with a printed note when `python3` is not on `PATH`.
7. Update `README.md` (one sentence on typed and untyped routers, linking `docs/routers.md`), the decree skill (when to use a typed router), `docs/reference/runs.md` and `tests/README.md`. Add a `docs/decisions.md` entry: "typed routers for routing, untyped models for the work", with the reasons above.

- Only this migration's scope.
- If the reference docs and the code disagree, or a case is not covered here, do not guess: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- Tests build their own `.decree/` in a temp directory and never touch this repository's `.decree/`. No test calls a real LLM or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass.

## Files to Modify

- docs/routers.md, docs/services.md, docs/reference/runs.md, docs/decisions.md, README.md, src/templates/skills/decree/
- src/interpreter/decide.rs (`reply_schema`)
- examples/route-by-complexity/ (new), examples/sort-documents/ (`gliner_router`)
- tests/

## Acceptance Criteria

- **Given** `docs/routers.md`
  **When** it is read
  **Then** it opens with the typed/untyped table, and every router section names its kind in its first sentence

- **Given** a `model` decision
  **When** decree writes `request.json`
  **Then** it has a `reply_schema` whose `event` enum is the options, and every recorded `reply.json` validates against its request's schema

- **Given** the GLiNER quick start
  **When** it is followed
  **Then** it is at most five commands, and the server code exists in exactly one file

- **Given** `examples/route-by-complexity/` with stubs
  **When** each of the five paths runs
  **Then** the run takes that path and ends as listed
