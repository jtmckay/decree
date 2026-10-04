---
machine: rust_develop
---
# 75: Example: services in tmux sessions (GLiNER, Ollama, ComfyUI)

## Overview

`docs/services.md` shows systemd units as the supervisor for long-running services, with `Conflicts=` for "never both on the GPU". Many people run these services in tmux instead, because they can attach to them and watch them. The user wants an example of that pattern:

- **On entry**, a state's `onentry` script attaches to the service's tmux session if it is running, or starts it in a new detached session, then waits until the service answers. "Attach" here means "use the running session". A script cannot take over a terminal; a person attaches with `tmux attach -t <session>`.
- **GLiNER** (CPU) runs alongside everything else.
- **ComfyUI and Ollama share the GPU**: starting ComfyUI ends Ollama, and starting Ollama ends ComfyUI.

decree still runs every script directly, never inside tmux ([scripts.md](../../docs/reference/scripts.md)). Only the services live in tmux.

## Requirements

Read `docs/services.md`, `docs/routers.md`, `examples/route-by-complexity/`, `examples/text-to-media/` and `tests/README.md` first.

1. **`examples/tmux-services/`**, a project that passes `decree check` and `decree graph`, with `.decree/schema/` and a README.
2. **One helper, sourced by the three `use_*` scripts**: `.decree/scripts/tmux_service.sh`. It defines three functions:
   - `ensure_session <session> <command>`: start `<command>` in a new detached tmux session named `<session>` unless `tmux has-session` finds it, and print which happened plus `tmux attach -t <session>`;
   - `end_session <session> <health url>`: kill the session if it exists, then wait up to 30 s for the health URL to stop answering. If it still answers, the service is running outside tmux (for example Ollama's own systemd unit): fail with a message that says so and how to stop it;
   - `wait_until_up <health url> <seconds>`: poll with `curl -fsS` once a second, and fail with a clear message on timeout.
   
   Every name, command, URL and timeout is a variable with a default at the top of the script that uses it, overridable from the environment.
3. **The three `onentry` scripts:**
   - `use_gliner`: session `gliner`. It runs the GLiNER server from `examples/route-by-complexity/gliner/decide_server.py`. Do not copy it: default `GLINER_SERVER` to that path relative to this example, so it works in the repository, and say in the README what to set when copying the example elsewhere. It waits on a new `GET /health` endpoint. Add that endpoint to `decide_server.py`: it answers `200 {"ok": true}` once the model is loaded, and the server only starts listening after loading, so a connection means ready. Allow up to 10 minutes on first start, because of the model download.
   - `use_ollama`: first `end_session comfyui …`, then `ensure_session ollama 'ollama serve'`, then wait on `http://127.0.0.1:11434/api/version`.
   - `use_comfyui`: first `end_session ollama …`, then `ensure_session comfyui` running ComfyUI's `main.py` in `COMFYUI_DIR` (default `$HOME/ComfyUI`) on `127.0.0.1:8188`, then wait on `/system_stats`.
   
   Verify Ollama's `/api/version`, ComfyUI's `/system_stats`, `/prompt` and `/history/<id>` against their docs or source before relying on them; fetching those pages is allowed.
4. **A machine that uses all three, `illustrated_post`**: write a short post from the message, with a picture when the message asks for one.
   - **Root `onentry: [use_gliner]`**: GLiNER is needed first, and stays up.
   - **`needs_picture`**: a `model` invoke with `router: gliner_router` (a copy of the machine and script from `examples/route-by-complexity/`, held identical by `tests/templates_test.rs` like the other copies). The question is "Does this post need a picture?", and the options are `with_picture` and `text_only`. `unsure` → `text_only`.
   - **`render`**: `onentry: [use_comfyui]`. It queues a ComfyUI workflow built from the message, reusing `examples/text-to-media/workflows/image_flux2_text_image.json` by path (a `COMFY_WORKFLOW` variable). It waits for the prompt to finish (`/history/<prompt_id>`) before it exits, because the next state's `use_ollama` ends ComfyUI, and must not cut a render short. It writes the image path to the run directory.
   - **`write`**: `onentry: [use_ollama]`. It asks Ollama's HTTP API (`/api/generate`, model in `OLLAMA_MODEL`, a current small model by default) for the post, and writes `post.md` to the run directory, linking the image if there is one.
   - **Paths**: `with_picture` → `render` → `write` → `done`; `text_only` → `write` → `done`. The graph then shows each switch as an `onentry` note.
5. **README:**
   - the pattern in a few lines;
   - what to install (tmux, curl, jq, Ollama, ComfyUI, `gliner2[local]`);
   - how to run it (`decree emit --machine illustrated_post`, `decree process`);
   - how to watch it (`tmux ls`, `tmux attach -t comfyui`, `decree tail`);
   - what to do when Ollama is also installed as a system service;
   - the trade-off against systemd: tmux gives you no restart on crash and no start at boot, but you can watch and use the service live.
   
   Link it from `docs/services.md`, in a short new section "tmux sessions as the supervisor" next to the systemd one.
6. **Tests, no network and no real services.** A stub `tmux` keeps sessions as files in a temp directory and records its calls. A stub `curl` answers health URLs according to which stub sessions exist, and answers ComfyUI's `/prompt` and `/history` and Ollama's `/api/generate`. A stub `ask_gliner` reply picks the path. Through the binary:
   - with no sessions, the `with_picture` path starts `gliner`, starts `comfyui` after ending `ollama` (absent, so nothing to end), then ends `comfyui` before starting `ollama`, and ends in `done` with `post.md` written;
   - the `text_only` path never touches `comfyui`;
   - a running `gliner` session is reused, not started again;
   - a service that never answers fails its state's `onentry`, and the run ends in `failed` with the timeout message;
   - a health URL that still answers after its session ended fails with the "running outside tmux" message;
   - `decide_server.py` still byte-compiles.

- Only this migration's scope.
- If the reference docs and the code disagree, or a case is not covered here, do not guess: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- Tests build their own `.decree/` in a temp directory and never touch this repository's `.decree/`. No test calls a real LLM, a real service or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass.

## Files to Modify

- examples/tmux-services/ (new)
- examples/route-by-complexity/gliner/decide_server.py (`GET /health`)
- docs/services.md, docs/routers.md (the health endpoint), tests/README.md, tests/examples_test.rs (the project list)
- tests/ (new test file)

## Acceptance Criteria

- **Given** the `with_picture` path with stubs
  **When** the run goes through `render` and then `write`
  **Then** the stub tmux log shows `comfyui` ended before `ollama` started, `ollama` ended before `comfyui` started, and `gliner` started once

- **Given** a `gliner` session that already exists
  **When** the run starts
  **Then** no new `gliner` session is started

- **Given** a service whose health URL never answers, or still answers after its session ended
  **When** its `onentry` runs
  **Then** the run ends `failed` with a message that says which service and why

- **Given** `examples/tmux-services/`
  **When** `decree check` and `decree graph` run
  **Then** check exits 0 and graph leaves `.decree/graph/` unchanged
