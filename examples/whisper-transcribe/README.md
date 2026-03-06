# Whisper Transcribe — Audio Transcription Pipeline

Transcribe audio files using [OpenAI Whisper](https://github.com/openai/whisper)
via decree migrations.

## What This Demonstrates

- **Non-AI routines** — decree routines don't have to invoke an AI; any shell
  script works
- **Custom parameters from frontmatter** — `input_file`, `output_file`, and
  `model` are passed as env vars from message frontmatter
- **File-based processing** — each migration references an audio file to
  transcribe

## How It Works

Each migration specifies an audio file path and optional parameters in
frontmatter. The `transcribe` routine calls Whisper to produce a `.txt`
transcription alongside the original file.

## Prerequisites

```bash
pip install -U openai-whisper
```

## Message Format

```yaml
---
routine: transcribe
input_file: ./audio/meeting-notes.mp3
model: base # optional, defaults to "large"
output_file: ./out.txt # optional, defaults to input with .txt extension
---
Transcribe description (ignored by routine, for human context).
```

## Usage

```bash
cd examples/whisper-transcribe
decree process
```

## Daemon Mode

Drop messages into `.decree/inbox/` for continuous processing:

```bash
decree daemon
```

External tools can write messages directly to the inbox directory and
decree will pick them up automatically.
