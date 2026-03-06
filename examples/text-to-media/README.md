# ComfyUI Media — Image & Video Generation Pipeline

Generate images and videos via [ComfyUI](https://github.com/comfyanonymous/ComfyUI)
REST API using decree migrations.

## What This Demonstrates

- **Multiple routines** — three routines for different generation modes
  (text-only, text+image, image-to-video)
- **Workflow templates** — ComfyUI JSON workflows in `workflows/` are
  patched with jq at runtime using parameters from frontmatter
- **Non-AI routines** — no AI assistant involved; routines call the
  ComfyUI API directly

## Routines

| Routine | Workflow | Description |
|---------|----------|-------------|
| `comfy-image-text` | FLUX2 text-to-image | Generate images from text prompts |
| `comfy-image-text-image` | FLUX2 text+image | Generate images guided by text and a reference image |
| `comfy-video-i2v` | WAN2.2 image-to-video | Animate a still image into video with text guidance |

## Message Format

### Text-to-Image

```yaml
---
routine: comfy-image-text
width: 800              # optional, default: 400
height: 400             # optional, default: 400
output_prefix: my_image # required — ComfyUI output filename prefix
---
Your image generation prompt goes here.
```

### Text + Reference Image

```yaml
---
routine: comfy-image-text-image
input_image: reference.png  # required — filename in ComfyUI's input dir
output_prefix: my_output    # required
---
Describe the desired output, referencing the input image.
```

### Image-to-Video

```yaml
---
routine: comfy-video-i2v
input_image: frame.png      # required — first frame image
output_prefix: my_video     # required
width: 640                  # optional, default: 640
height: 640                 # optional, default: 640
---
Describe the motion and scene for the video.
```

## Prerequisites

- A running [ComfyUI](https://github.com/comfyanonymous/ComfyUI) instance
  (default: `http://127.0.0.1:8288`)
- FLUX2 and/or WAN2.2 models loaded in ComfyUI
- `curl` and `jq` installed

## Usage

```bash
cd examples/comfyui-media
decree process
```

## Daemon Mode

Drop messages into `.decree/inbox/` for continuous generation:

```bash
decree daemon
```

This is particularly useful when integrating with external tools (game
engines, web apps) that produce generation requests programmatically.
