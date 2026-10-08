---
machine: develop
---
# 97: Guidance from real runs: routers, services, runs, child machines

## Overview

Documentation and examples only. Each item below was learned running decree 0.5 at work.

## Requirements

1. **Reason before event in `reply_schema`** (code change, small). decree builds `request.json`'s `reply_schema` with `event` before `reason`. Constrained decoders (llama.cpp, Ollama `format`) emit properties in schema order, so the model commits to a pick before it reasons: Qwen picked the first option for every request, contradicting its own reason. Put `reason` first in `properties` (and keep `required` unchanged). Update `request.schema.json` if it shows the order, the example `request.json` in `docs/reference/runs.md`, and add a short note in `docs/routers.md`: when a schema-constrained model routes, put the reasoning field first.
2. **`docs/routers.md`, "Where GLiNER fits"**, from measured use:
   - good when the answer shows in the wording (ComfyUI methods: 16 of 18 right after splitting the options);
   - poor at judging (every spec came out "routine" at 0.92 or more);
   - confidence spreads thin across many labels (0.25–0.35 for correct picks among 7);
   - with two options the winner is always at least 0.5, so a `min_confidence` floor filters almost nothing;
   - so: keep each decision to a few options, and settle anything the params already answer with `check` states first.
3. **A schema-constrained local router** beside `gliner_router` in `examples/route-by-complexity/`: `llm_router.yml` and its script, asking Ollama's `/api/chat` with `format` set to the request's `reply_schema` (works with llama.cpp's server too; say how in a comment). The README says when to use which, in two sentences. Keep the machine itself unchanged unless it needs a one-line `router:` comment.
4. **GLiNER install pitfalls** in the example README: the `gliner2` package alone is only the cloud API client; running the model locally needs `gliner2[local,train]`, since the runtime imports training modules too. (Check `decide_server.py`'s install line and the tmux README too.)
5. **Prepare shared resources in an invoke with a `timeout`, not `onentry`.** `onentry` scripts have no timeout, and a prep step that waits (for ComfyUI's queue, for a model to unload) can hang. `docs/services.md`: recommend an invoked prep state (`prep_gpu: { invoke: { script: { name: use_comfy, timeout: 1h } } }`) whenever the prep waits; `onentry` only for quick ones. Update `examples/tmux-services` to match: the waiting `without_comfy_wait` becomes an invoked state with a timeout; the quick ones stay `onentry`.
6. **Child machines: a few generic outcomes.** V8 makes every caller handle every final state of a child machine. `docs/reference/machines.md` (composition): keep a child machine's final states few and generic (`resolved`, `answered`, `failed`), since each one is a transition every caller must have.
7. **Batching by resource** (`docs/reference/messages.md`, Migrations): migration N's emitted messages run before migration N+1 starts, so a migration that does the LLM work and emits the GPU work naturally batches all LLM work, then all ComfyUI work.
8. **`examples/text-to-media`:** the workflow files move to `.decree/lib/comfy/` (data scripts read through `$DECREE_LIB`), and the "When messages stop naming a method" fragment uses one `build` script with `env: { METHOD: … }` per state instead of state-name parsing, and `enum` on `method` (migrations 94 and 95). Keep `tmux-services`' workflow path working.
9. **The skill** (SKILL.md rules, short):
   - "Shared code, config and data go in `.decree/lib/` (`$DECREE_LIB`); `scripts/` holds only what states invoke. Project config goes in `.decree/env`; per-state values in the invoke's `env:`."
   - "Never delete run folders to clean up; they are the record. Use `decree prune --older-than <age>`."
   - "A step that waits for a shared resource is an invoked state with a `timeout`."
10. CHANGELOG entries (Changed for item 1, the rest under a single "Docs" line in Changed).

- Only this migration's scope.
- Never edit `.decree/migrations/` or `.decree/runs/`, not even by a search and replace across the repository: exclude both from every bulk edit.
- If the reference docs and this migration disagree in a way you cannot settle, write the question to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- No test calls a real model or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass, and `decree check` passes in every example directory.

## Acceptance Criteria

- **Given** a `model` state
  **When** decree writes `request.json`
  **Then** `reply_schema.properties` lists `reason` before `event`

- **Given** `examples/route-by-complexity`
  **When** `decree check` runs
  **Then** it passes with `llm_router.yml` present, and the README says when to use each router

- **Given** `examples/tmux-services`
  **When** its machine is read
  **Then** the ComfyUI drain-and-unload step is an invoked state with a `timeout`

- **Given** the skill
  **When** it is read
  **Then** it has the `lib/`, `env`, "never delete runs" and "prep with a timeout" rules
