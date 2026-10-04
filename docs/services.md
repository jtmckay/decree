# Long-running services

Model servers, ComfyUI and decision models such as GLiNER2.5-Decide run for hours and are shared by many runs. They are not scripts, and decree does not manage them. This guide shows how to run them so decree can use them, how to switch a GPU between them, and how to watch everything.

Nothing here is required. decree does not depend on systemd, llama-swap, Docker, NVIDIA tools or tmux; they are examples of how one setup might look. The only mechanism decree provides is `onentry`: a script of yours that runs before a state does its work, and that can prepare whatever the state needs in whatever way suits your machine.

The rule of thumb:

- A **service** runs on its own, under a supervisor (systemd, llama-swap, Docker).
- A **machine** says when it needs one, with an `onentry` script that starts it and waits until it answers. The switch then shows up in the machine, its graph and its events, like any other step.

## Pick a supervisor

| Situation | Use |
| --- | --- |
| The things you swap all speak HTTP and you reach them through one address | **llama-swap**: loads the server for the requested model, unloads the previous one |
| Anything else, or you want restarts, logs and "never both at once" for any process | **systemd user units** with `Conflicts=` |
| You want to watch and use the services live, and need no restarts or start at boot | **tmux sessions**, started and ended by the `onentry` scripts |
| decree itself runs in a container without systemd | **Docker Compose** services, started and stopped by the `onentry` scripts |

## llama-swap: hot-swapping model servers

[llama-swap](https://github.com/mostlygeek/llama-swap) (MIT) is a proxy that starts the server for the model named in each OpenAI-compatible request, and stops the one that was running. It runs llama.cpp, vLLM and other OpenAI-compatible servers, and its README lists ComfyUI among supported upstreams. Unloading happens on swap, after a `ttl`, or through `POST /api/models/unload`.

```yaml
# llama-swap config
models:
  big-llm:
    cmd: llama-server --port ${PORT} --model /models/big.gguf
    ttl: 900                       # unload after 15 idle minutes, freeing the VRAM
```

Point a router machine at llama-swap's address. Asking for `big-llm` loads it; asking for another model swaps. decree needs no `onentry` script for this, because the swap happens on the request.

## systemd user units: "only one of these at a time"

`Conflicts=` makes two units mutually exclusive: starting one stops the other. `Restart=` brings a crashed service back, and journald keeps its logs.

```ini
# ~/.config/systemd/user/llm.service
[Unit]
Description=Large model server (uses the GPU)
Conflicts=comfyui.service

[Service]
ExecStart=%h/bin/serve-llm
Restart=on-failure
```

```ini
# ~/.config/systemd/user/comfyui.service
[Unit]
Description=ComfyUI (uses the GPU)
Conflicts=llm.service

[Service]
WorkingDirectory=%h/comfy/ComfyUI
ExecStart=%h/comfy/ComfyUI/run.sh
Restart=on-failure
```

```ini
# ~/.config/systemd/user/decide.service
[Unit]
Description=GLiNER2.5-Decide on CPU, for model routers

[Service]
# examples/route-by-complexity/gliner/decide_server.py, copied to ~/gliner/: loads the
# model once and serves POST /classify and GET /health on 127.0.0.1:8090 (docs/routers.md)
ExecStart=%h/.local/bin/uv run --with gliner2[local] python %h/gliner/decide_server.py
Restart=on-failure
```

`decide` has no `Conflicts=`, so it keeps running while the GPU switches. Load the unit files with `systemctl --user daemon-reload`, and run `loginctl enable-linger $USER` once so user services keep running when you log out.

Useful commands:

```bash
systemctl --user list-units 'llm*' 'comfyui*' 'decide*'   # what is up
systemctl --user start comfyui                          # also stops llm
journalctl --user -u comfyui -f                         # its log, live
```

## tmux sessions as the supervisor

If you would rather watch the services live, run each in a tmux session named after it. The `onentry` script uses the session if it is running, or starts the service in a new detached one, then waits until it answers; for "never both on the GPU", it first ends the other service's session. [`examples/tmux-services/`](../examples/tmux-services/README.md) does this for GLiNER2.5-Decide (on CPU, always up), Ollama and ComfyUI (which end each other), with one sourced helper for the three scripts. Jobs are queued without waiting: before `use_ollama` ends ComfyUI, a `wait_for_empty` script polls ComfyUI's `GET /queue` until nothing is running or pending, so no render is cut short. tmux restarts nothing after a crash and starts nothing at boot, but you can `tmux attach -t comfyui` and watch or use the service as it runs.

## Using a service from a machine

An `onentry` script starts what the state needs and waits until it answers:

```bash
#!/usr/bin/env bash
# scripts/use_comfyui.sh: start ComfyUI (systemd stops the LLM to free the VRAM),
# then wait up to 3 minutes for it to answer.
set -euo pipefail
systemctl --user start comfyui
for _ in $(seq 180); do
  curl -fsS http://127.0.0.1:8188/ >/dev/null 2>&1 && exit 0
  sleep 1
done
echo "comfyui did not answer within 180 s" >&2
exit 1
```

```yaml
generate_images:
  onentry: [use_comfyui]          # a failure here is an error, like any onentry
  invoke: render
  transitions: { done: describe_images }
describe_images:
  onentry: [use_llm]              # swaps the GPU back
  invoke: describe
  transitions: { done: done }
```

Each switch is a timed `script` event, so Grafana shows how long swaps take. decree runs one run at a time per process, so one `decree daemon` never asks for two GPU services at once.

## Docker Compose

If decree runs in a container, systemd is usually not there. Run the services as Compose services and let the `onentry` scripts stop one before starting the other (`docker compose stop llm && docker compose start comfyui`), then wait for the port as above.

## Watching it all with tmux

decree runs scripts directly, not inside tmux: it needs their exact exit codes, separate stdout and stderr, and no terminal. Live visibility comes from `decree status` (what runs now, its pid, elapsed time and log path) and `decree tail` (the live log). tmux is a good personal dashboard on top:

```bash
#!/usr/bin/env bash
# decree-dash: attach to the dashboard, or create it.
set -euo pipefail
session=decree
if tmux has-session -t "$session" 2>/dev/null; then
  exec tmux attach-session -t "$session"
fi
tmux new-session -d -s "$session" -n runs 'watch -n 5 decree status'
tmux split-window -t "$session":runs -v 'while true; do decree tail; sleep 5; done'
tmux new-window -t "$session" -n services 'journalctl --user -f -u llm -u comfyui -u decide'
tmux new-window -t "$session" -n gpu 'nvidia-smi --query-gpu=timestamp,utilization.gpu,memory.used,temperature.gpu,power.draw --format=csv --loop=5'
exec tmux attach-session -t "$session"
```
