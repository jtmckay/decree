---
machine: rust_develop
---
# 76: ComfyUI jobs run in the background; wait_for_empty before ending ComfyUI

## Overview

ComfyUI's API is fire and forget: `POST /prompt` queues a job and returns its id. Decided by the user: queuing stays non-blocking, and the only wait is right before something ends ComfyUI. In `examples/tmux-services/`, that is `use_ollama`.

## Requirements

1. `illustrated_post/render.sh` queues the workflow, appends the prompt id to `comfy-prompts.txt` in the run directory, and returns.
2. A new flat script, `wait_for_empty.sh`, runs as `write`'s first `onentry` script, before `use_ollama`:
   - it polls `GET /queue` until `queue_running` and `queue_pending` are both empty (as in ComfyUI's `server.py`), within `COMFY_DRAIN_TIMEOUT_S`;
   - then, while ComfyUI still holds its history, it writes this run's images to `images.txt`;
   - it fails if a prompt failed, if a prompt is in neither the queue nor the history (lost, for example by a restart), or if no image was saved;
   - if ComfyUI is not running, it returns at once, unless this run queued prompts.
3. `write.sh` links every image in `images.txt`.
4. Update the README, the graph and `docs/services.md`. Test the success path (the queue is polled, the images are collected) and a failed or lost prompt (the run fails in `wait_for_empty`, and ComfyUI is not ended).
