#!/usr/bin/env bash
# onentry: end the ollama tmux session and wait until Ollama stops answering,
# freeing the GPU. No wait for work first: decree calls Ollama synchronously
# (a script waits for its answer), so by the time a state runs this, nothing
# from decree is using Ollama.
set -euo pipefail
OLLAMA_SESSION="${OLLAMA_SESSION:-ollama}"
OLLAMA_HEALTH="${OLLAMA_HEALTH:-http://127.0.0.1:11434/api/version}"
source "$(dirname "${BASH_SOURCE[0]}")/tmux_service.sh"

end_session "$OLLAMA_SESSION" "$OLLAMA_HEALTH"
