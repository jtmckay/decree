---
machine: rust_develop
---
# 78: Keep the servers up; `without_*` unloads models through their APIs

## Overview

In `examples/tmux-services/`, `without_ollama` and `without_comfy_*` end the other service's tmux session to free the GPU. Restarting a server is the slow part of a swap, and killing a process loses its queue and history. Decided by the user: keep both servers running, and free the VRAM by unloading models through each API. Every API fact below was read from the source or the docs (2026-10-04).

**Ollama** (https://docs.ollama.com/api/ps, https://docs.ollama.com/faq):

- `GET /api/ps` lists the loaded models (`models[].name`, `size_vram`).
- `POST /api/generate` with `{"model": <name>, "keep_alive": 0}` unloads that model.
- VRAM is free once `/api/ps` lists no models.

**ComfyUI** (`server.py`, `main.py` and `execution.py` on its master branch):

- `POST /free` with `{"unload_models": true, "free_memory": true}` sets queue flags, and `set_flag` wakes the worker. Between jobs, the worker calls `unload_all_models()`, `e.reset()`, `gc.collect()` and `soft_empty_cache()`. Jobs still queued would load their models again, so a full unload needs an empty queue.
- `POST /queue` with `{"clear": true}` empties the pending queue, and `POST /interrupt` stops the running job.
- `GET /system_stats` reports `devices[].torch_vram_total`, the memory PyTorch has reserved. It drops once the cache is emptied. The process keeps its CUDA context, a few hundred MB, which `/free` cannot release.

## Requirements

1. **The `use_*` scripts start nothing that already answers.** `use_gliner`, `use_ollama` and `use_comfy` first check the service's health URL. If it answers, they use it, whoever runs it (a tmux session, Ollama's own systemd service, anything), and print that. Otherwise they `ensure_session` and `wait_until_up` as today. A service run by systemd then works without any change.
2. **`without_ollama`** unloads every model `/api/ps` lists with `keep_alive: 0`, then polls `/api/ps` until it lists none, within `OLLAMA_UNLOAD_TIMEOUT_S` (default 60). If Ollama does not answer, there is nothing to unload: exit 0. On timeout, fail and name the models still loaded. The session stays up.
3. **`without_comfy_no_wait`** posts `/queue {"clear": true}`, then `/interrupt`, then `/free {"unload_models": true, "free_memory": true}`. It then polls `/system_stats` until every device's `torch_vram_total` is at most `COMFY_RESERVED_MAX_MB` (default 1024), within `COMFY_UNLOAD_TIMEOUT_S` (default 60). If ComfyUI does not answer, exit 0. On timeout, fail with the reserved amount. The session stays up.
4. **`without_comfy_wait`** keeps what it does today (wait until `/queue` is empty, then collect this run's images, failing on a failed or lost prompt), then runs `without_comfy_no_wait`. The queue is empty by then, so clearing and interrupting do nothing. It no longer ends the session.
5. **`tmux_service.sh`** loses `end_session` and `TMUX_END_TIMEOUT_S`; nothing uses them now.
6. **Docs.**
   - `examples/tmux-services/README.md`:
     - the pattern (`use_<service>` starts it if it is not answering; `without_<service>` unloads its models, and the server keeps running);
     - the variables;
     - what is and is not freed (the CUDA context);
     - replace the "When Ollama is also a system service" section with one line: it works, because `use_ollama` uses whatever answers.
   - `docs/services.md`'s tmux section: say that freeing the GPU unloads models through the APIs, so the servers stay up, and keep the API references.
7. **Tests**, rewritten for the new behaviour, still with stub `tmux` and `curl`. The stub `curl` keeps "loaded" state as files: `/prompt` loads ComfyUI's model, `/free` unloads it, a plain `/api/generate` loads the Ollama model, and `keep_alive: 0` unloads it. `/api/ps` and `/system_stats` report from those files. Cases:
   - on the `with_picture` path, both sessions are started once and never killed. ComfyUI's model is unloaded before Ollama's request, and the `/free` body is exactly `{"unload_models": true, "free_memory": true}`;
   - with Ollama's model already loaded (a second picture run), `without_ollama` unloads it with `keep_alive: 0` before `/prompt`;
   - a service that already answers without a tmux session is used, and no session is started;
   - an unload that never takes effect fails the state with the timeout message;
   - a failed or lost prompt still fails `without_comfy_wait` before anything is unloaded;
   - `without_comfy_no_wait` clears the queue and interrupts before `/free`.

- Only this migration's scope.
- If the reference docs and the code disagree, or a case is not covered here, do not guess: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- Tests build their own `.decree/` in a temp directory and never touch this repository's `.decree/`. No test calls a real LLM, a real service or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass.

## Files to Modify

- examples/tmux-services/.decree/scripts/ (use_*, without_*, tmux_service.sh), its machine comment, README.md and graph
- docs/services.md
- tests/tmux_services_test.rs

## Acceptance Criteria

- **Given** the `with_picture` path with stubs
  **When** the run finishes
  **Then** no tmux session was killed, ComfyUI's model was unloaded through `/free` before Ollama's request, and both sessions still exist

- **Given** an Ollama model already loaded
  **When** `render`'s `without_ollama` runs
  **Then** it posts `keep_alive: 0` for that model and waits until `/api/ps` lists none

- **Given** a service that already answers without a tmux session
  **When** its `use_*` script runs
  **Then** no session is started

- **Given** an unload that never takes effect
  **When** its `without_*` script runs
  **Then** the state fails with a message naming the service and what is still loaded
