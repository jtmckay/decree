# Whisper Transcribe — Audio Transcription Pipeline

Transcribe audio files using [OpenAI Whisper](https://github.com/openai/whisper)
via decree migrations.

## What This Demonstrates

- **Machines without AI agents** — a machine's scripts can be any program;
  here they call Whisper
- **Parameters from the message** — `input_file`, `output_file` and `model`
  are the machine's `data`, set per message with `params` and read by the
  scripts as `DECREE_DATA_INPUT_FILE`, `DECREE_DATA_OUTPUT_FILE` and
  `DECREE_DATA_MODEL`
- **File-based processing** — each migration names an audio file to
  transcribe

## How It Works

Each migration names an audio file and optional settings in `params`. The
`transcribe` machine ([`.decree/machines/transcribe.yml`](.decree/machines/transcribe.yml),
drawn in [`.decree/graph/transcribe.md`](.decree/graph/transcribe.md)) runs
two scripts from `.decree/scripts/`:

1. `precheck` — fails fast if `whisper` or the input file is missing
2. `transcribe` — calls Whisper and writes a `.txt` transcription next to
   the original (or to `output_file`)

## Prerequisites

```text
pip install -U openai-whisper
```

The sample migration transcribes `audio/meeting-notes.mp3`; put a recording
there first.

## Message Format

```yaml
---
machine: transcribe
params:
  input_file: ./audio/meeting-notes.mp3
  model: base             # optional, default: large
  output_file: ./out.txt  # optional, default: input with a .txt extension
---
Transcribe description (ignored by the scripts, for human context).
```

## Usage

```bash
cd examples/whisper-transcribe
decree check
decree process
decree status
```

## Daemon Mode

Queue messages in `.decree/inbox/` for continuous processing:

```bash
echo "Transcribe the meeting notes." | decree emit --machine transcribe --param input_file=./audio/meeting-notes.mp3
decree daemon
```

External tools can queue messages the same way (or write `.decree/inbox/<name>.md`
through a temp file and rename), and decree picks them up automatically.
