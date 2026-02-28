# Historical Portraits — Chain-Based Pipeline

Generate AI art portraits of historical figures at pivotal moments using a
three-step chain: **research** → **prompt-craft** → **generate**.

## What This Demonstrates

- **Message chaining** — each routine spawns the next step automatically
- **Custom parameters** passed through spec frontmatter and chain messages
- **External API integration** — the final step submits to a ComfyUI server
- **Inter-step data** — routines share results via `$message_dir`

## How It Works

Each spec defines a historical figure, a specific moment, and an art style.
Processing a spec triggers a 3-step chain:

1. **research.sh** — Uses AI to research the person's appearance at that
   moment in history. Writes `research.json` to `$message_dir`.
2. **prompt-craft.sh** — Reads the research, crafts SDXL-optimized positive
   and negative prompts. Writes `prompt.json` to `$message_dir`.
3. **generate.sh** — Parses the prompt JSON, builds a ComfyUI SDXL workflow,
   submits via REST API, polls for completion, downloads the portrait.

## Specs

| Spec | Subject | Moment | Style |
|------|---------|--------|-------|
| 01 | Napoleon Bonaparte | Battle of Austerlitz, 1805 | Neoclassical oil painting |
| 02 | Cleopatra VII | Arriving in Rome, 46 BC | Pre-Raphaelite oil painting |
| 03 | Abraham Lincoln | Gettysburg Address, 1863 | Wet plate collodion photograph |

## Prerequisites

- A running [ComfyUI](https://github.com/comfyanonymous/ComfyUI) instance
  with an SDXL checkpoint loaded (default: `http://127.0.0.1:8188`)
- `curl` and `jq` installed
- An AI backend configured in `.decree/config.yml`

## Usage

```bash
cd examples/historical-portraits
decree process
```

Each spec produces a `portrait.png` in its run's message directory.
