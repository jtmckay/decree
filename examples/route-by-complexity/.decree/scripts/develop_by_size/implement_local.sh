#!/usr/bin/env bash
# develop_by_size's implement_local: a local model served by Ollama implements
# the message, through OpenCode. `opencode run --model <provider>/<model>` runs
# one prompt without the TUI (https://opencode.ai/docs/cli/); the `ollama`
# provider and this model must be in opencode.json, as the example's README
# shows (https://opencode.ai/docs/providers/). Install the model with
# `ollama pull qwen3-coder:30b`.
set -euo pipefail
MODEL="${OLLAMA_MODEL:-qwen3-coder:30b}"

prompt="Read ${DECREE_MESSAGE} and implement it. It is a small, mechanical change:
change only what it asks for, and keep the tests passing."
echo "=== opencode run --model ollama/${MODEL} ==="
echo "${prompt}"
opencode run --model "ollama/${MODEL}" "${prompt}"
# a plain last line: the model's last line must not be read as an event
echo "implemented with ollama/${MODEL}"
