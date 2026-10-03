# Text-to-Speech — Chatterbox TTS Pipeline

Convert text to speech using [Chatterbox TTS](https://github.com/devnen/Chatterbox-TTS-Server)
via decree migrations.

## What This Demonstrates

- **External API integration** — a script calls a local TTS server via REST
- **Rich parameters** — voice, temperature, exaggeration and other synthesis
  knobs are the machine's `data`, set per message with `params`
- **One script per step** — check the tools, synthesize a WAV, convert it to
  MP3 with ffmpeg; each step is a state with its own log

## How It Works

Each migration holds the text in its body and synthesis settings in
`params`. The `tts` machine ([`.decree/machines/tts.yml`](.decree/machines/tts.yml),
drawn in [`.decree/graph/tts.md`](.decree/graph/tts.md)) runs three scripts
from `.decree/scripts/`:

1. `precheck` — fails fast if `curl`, `ffmpeg` or `jq` is missing, or no
   `filename` was given
2. `synthesize` — sends the body to the Chatterbox server and saves the WAV
   in the run folder
3. `convert` — converts the WAV to `output/<filename>.mp3`

Scripts read the settings from `DECREE_DATA_<NAME>` variables.

## Prerequisites

```text
# Required tools
sudo apt install curl ffmpeg jq   # Debian/Ubuntu
brew install curl ffmpeg jq       # macOS

# Chatterbox TTS Server
conda create -n chatterbox python=3.11 -y && conda activate chatterbox
git clone git@github.com:devnen/Chatterbox-TTS-Server.git
cd Chatterbox-TTS-Server
chmod +x start.sh && ./start.sh
python server.py   # runs at http://localhost:8004
```

## Message Format

```yaml
---
machine: tts
params:
  filename: my-audio             # required — output file name (without extension)
  predefined_voice_id: Emily.wav # optional, default: Emily.wav
  temperature: "0.8"             # optional, default: "0.8" (quoted: data is string, int or bool)
  exaggeration: "1.3"            # optional, default: "1.3"
  seed: 3000                     # optional, default: 3000
---
The text to be spoken goes in the message body.
```

`decree check` rejects a param the machine does not declare, or one of the
wrong type. The full list, with defaults, is the `data` in `tts.yml`.

## Usage

```bash
cd examples/text-to-speech
decree check
decree process
decree status
```

Output audio files are saved to `output/{filename}.mp3`. Each run's logs are
in `.decree/runs/<migration>/`.

## Daemon Mode

Queue a message in `.decree/inbox/` for continuous processing:

```bash
echo "Speak this sentence." | decree emit --machine tts --param filename=sentence
decree daemon
```
