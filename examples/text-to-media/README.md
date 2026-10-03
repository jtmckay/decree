# ComfyUI Media — Image & Video Generation Pipeline

Generate images and videos via [ComfyUI](https://github.com/comfyanonymous/ComfyUI)
REST API using decree migrations.

## What This Demonstrates

- **Multiple machines** — three machines for different generation modes
  (text-only, text+image, image-to-video)
- **Shared and per-machine scripts** — `precheck` and `queue_prompt` live once
  in `.decree/scripts/` and serve all three machines; each machine's own
  `build_payload` lives in `.decree/scripts/<machine>/`, which decree checks
  first
- **Workflow templates** — ComfyUI JSON workflows in `workflows/` are
  patched with jq at runtime using the message's `params`
- **Machines without AI agents** — no AI assistant involved; the scripts call
  the ComfyUI API directly

## Machines

| Machine | Workflow | Description |
|---------|----------|-------------|
| `comfy_image_text` | FLUX2 text-to-image | Generate images from text prompts |
| `comfy_image_text_image` | FLUX2 text+image | Generate images guided by text and a reference image |
| `comfy_video_i2v` | WAN2.2 image-to-video | Animate a still image into video with text guidance |

Each runs `precheck` (curl and jq are installed), `build_payload` (patch the
workflow into `runs/<id>/comfy-payload.json`) and `queue_prompt` (post it,
save the reply as `runs/<id>/comfy-response.json`). The graphs are in
[`.decree/graph/`](.decree/graph/system.md).

## Message Format

### Text-to-Image

```yaml
---
machine: comfy_image_text
params:
  width: 800              # optional, default: 400
  height: 400             # optional, default: 400
  output_prefix: my_image # required — ComfyUI output filename prefix
---
Your image generation prompt goes here.
```

### Text + Reference Image

```yaml
---
machine: comfy_image_text_image
params:
  input_image: reference.png  # required — filename in ComfyUI's input dir
  output_prefix: my_output    # required
---
Describe the desired output, referencing the input image.
```

### Image-to-Video

```yaml
---
machine: comfy_video_i2v
params:
  input_image: frame.png      # required — first frame image
  output_prefix: my_video     # required
  width: 640                  # optional, default: 640
  height: 640                 # optional, default: 640
---
Describe the motion and scene for the video.
```

Every machine also takes `api_url` (default: `http://127.0.0.1:8288/api/prompt`).

## Prerequisites

- A running [ComfyUI](https://github.com/comfyanonymous/ComfyUI) instance
  (default: `http://127.0.0.1:8288`)
- FLUX2 and/or WAN2.2 models loaded in ComfyUI
- `curl` and `jq` installed

## Usage

```bash
cd examples/text-to-media
decree check
decree process
decree status
```

## Daemon Mode

Queue messages in `.decree/inbox/` for continuous generation:

```bash
echo "A lighthouse at dawn, oil painting" | decree emit --machine comfy_image_text --param output_prefix=lighthouse
decree daemon
```

This is particularly useful when integrating with external tools (game
engines, web apps) that produce generation requests programmatically.
