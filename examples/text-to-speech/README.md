# Text-to-Speech — Chatterbox TTS Pipeline

Convert text to speech using [Chatterbox TTS](https://github.com/devnen/Chatterbox-TTS-Server)
via decree migrations.

## What This Demonstrates

- **External API integration** — routine calls a local TTS server via REST
- **Rich custom parameters** — voice, speed, temperature, and other synthesis
  knobs controlled via frontmatter
- **Audio format conversion** — WAV to MP3 via ffmpeg as a post-processing step

## How It Works

Each migration contains text in its body and synthesis parameters in
frontmatter. The `tts` routine sends the text to the Chatterbox server,
receives a WAV file, converts it to MP3, and saves it to `output/`.

## Prerequisites

```bash
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
routine: tts
filename: my-audio # required — output filename (without extension)
predefined_voice_id: Emily.wav # optional, default: Emily.wav
temperature: 0.8 # optional, default: 0.8
exaggeration: 1.3 # optional, default: 1.3
seed: 3000 # optional, default: 3000
---
The text to be spoken goes in the message body.
```

## Usage

```bash
cd examples/text-to-speech
decree process
```

Output audio files are saved to `output/{filename}.mp3`.

## Daemon Mode

Drop messages into `.decree/inbox/` for continuous processing:

```bash
decree daemon
```
