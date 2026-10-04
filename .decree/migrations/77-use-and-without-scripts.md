---
machine: rust_develop
---
# 77: `use_<service>` and `without_<service>` scripts

## Overview

The user asked for scripts named for what they do, with starting a service kept separate from freeing the GPU:

- **`use_<service>`** uses or starts the service's tmux session and waits until it answers. It ends nothing.
- **`without_<service>`** ends the service's session and waits until it stops answering. There is one for Ollama and two for ComfyUI:
  - **`without_ollama`** ends Ollama at once. decree calls Ollama synchronously (a script waits for its answer), so by the time a state that needs ComfyUI starts, nothing from decree is using Ollama.
  - **`without_comfy_wait`** waits until ComfyUI's queue is empty, collects this run's images, then ends ComfyUI. ComfyUI's API is fire and forget, so jobs queued earlier may still be rendering. On a failed or lost job it fails, and leaves ComfyUI running.
  - **`without_comfy_no_wait`** ends ComfyUI at once, for when its queued jobs no longer matter.

## Requirements

1. In `examples/tmux-services/`:
   - `use_comfyui` → `use_comfy`, and neither `use_*` script ends anything;
   - `wait_for_empty` → `without_comfy_wait`, which also ends ComfyUI;
   - new scripts `without_ollama` and `without_comfy_no_wait`.
2. `illustrated_post`: `render` gets `onentry: [without_ollama, use_comfy]`, and `write` gets `onentry: [without_comfy_wait, use_ollama]`.
3. Update the README, its graph and `docs/services.md`.
4. Tests:
   - the switch order is unchanged;
   - a service that still answers after its session ended fails in `without_ollama`;
   - a failed or lost prompt fails in `without_comfy_wait` and leaves ComfyUI running;
   - `without_comfy_no_wait` ends ComfyUI without asking for the queue.
