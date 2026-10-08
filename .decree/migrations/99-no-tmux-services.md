---
machine: develop
---
# 99: Remove the `tmux-services` example; free VRAM through the services' APIs

## Overview

The user finds the tmux examples no longer useful. GPU sharing is done by keeping the servers running and unloading their models through each API, so supervising services in tmux sessions adds nothing. What is worth keeping from `examples/tmux-services/` is the knowledge of the unload calls and when they are safe, and `docs/services.md` already holds most of it.

## Requirements

1. **Delete** `examples/tmux-services/` and `tests/tmux_services_test.rs` (`git rm -r`).
2. **`docs/services.md`:**
   - Remove the tmux supervisor material: the "tmux sessions" row of the choice table and the section "tmux sessions as the supervisor".
   - Keep "Watching it all with tmux": it is about watching decree, not about running services.
   - Add a short section, **"Freeing VRAM through the APIs"**, that keeps what the example knew:
     - the servers stay up, and only the models are unloaded, so a swap costs a model load, not a server start;
     - Ollama: `GET /api/ps` lists the loaded models, `POST /api/generate {"model": <name>, "keep_alive": 0}` unloads one, and the VRAM is free once `/api/ps` lists none. decree waits on its scripts, so nothing decree runs is using Ollama when another state starts;
     - ComfyUI's API is fire and forget, so first wait until `GET /queue` shows nothing running or pending; a queued job would load its models again. Then `POST /free {"unload_models": true, "free_memory": true}`, and poll `GET /system_stats` until `torch_vram_total` drops. The process keeps its CUDA context, a few hundred MB;
     - a service that does not answer has nothing loaded;
     - the step that waits is an invoked state with a `timeout`, not an `onentry` script (as the section already recommends).
   - Every link to `examples/tmux-services/` goes, or points at this section.
3. **Every other reference** to `tmux-services` or `tmux_services`, outside `.decree/migrations/`, `.decree/runs/` and the CHANGELOG's earlier entries: `docs/routers.md`, `examples/route-by-complexity/` (README, and the comment in `gliner/decide_server.py`), `examples/text-to-media/README.md` (`COMFY_URL` needs no other example to explain it), `tests/examples_test.rs`, `tests/templates_test.rs` (the byte-identical `gliner_router` copies: drop the tmux copy, keep `tests/fixtures/escalation`), `tests/README.md`, and the main README's examples list.
4. **CHANGELOG.md**, under Removed: the `tmux-services` example; its API unloading is in `docs/services.md`.

- Only this migration's scope. No change to decree's behaviour.
- Never edit `.decree/migrations/` or `.decree/runs/`, not even by a search and replace across the repository: exclude both from every bulk edit.
- If anything here contradicts the reference docs in a way you cannot settle, write the question to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- No test calls a real model or the network. No new dependencies.
- Print the evidence for each acceptance criterion at the end of your reply.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass, and `decree check` passes in every example directory.

## Acceptance Criteria

- **Given** the repository
  **When** `rg -n --hidden 'tmux-services|tmux_services' --glob '!.decree/migrations/**' --glob '!.decree/runs/**' --glob '!.git/**' --glob '!CHANGELOG.md'` runs
  **Then** nothing matches

- **Given** `docs/services.md`
  **When** it is read
  **Then** it has no tmux supervisor section, keeps "Watching it all with tmux", and its "Freeing VRAM through the APIs" section gives the Ollama and ComfyUI calls, the queue drain before `/free`, and the invoked state with a `timeout`

- **Given** `examples/`
  **When** it is listed
  **Then** it holds `newsletter`, `observability`, `project`, `route-by-complexity` and `text-to-media`
