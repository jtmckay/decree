#!/usr/bin/env bash
# onentry: unload every model Ollama has loaded, freeing the GPU; the server
# keeps running. GET /api/ps lists the loaded models; POST /api/generate with
# {"model": <name>, "keep_alive": 0} unloads one; the VRAM is free once
# /api/ps lists none (https://docs.ollama.com/api/ps, https://docs.ollama.com/faq).
# No wait for work first: decree calls Ollama synchronously (a script waits for
# its answer), so by the time a state runs this, nothing from decree is using it.
set -euo pipefail
OLLAMA_URL="${OLLAMA_URL:-http://127.0.0.1:11434}"
OLLAMA_UNLOAD_TIMEOUT_S="${OLLAMA_UNLOAD_TIMEOUT_S:-60}"

# The loaded models, one per line; fails if Ollama does not answer.
loaded() {
  local ps
  ps=$(curl -fsS --max-time 5 "$OLLAMA_URL/api/ps" 2>/dev/null) || return 1
  jq -r '.models[]?.name' <<<"$ps"
}

if ! models=$(loaded); then
  echo "ollama does not answer at $OLLAMA_URL: nothing to unload"
  exit 0
fi
while read -r model; do
  [ -n "$model" ] || continue
  curl -fsS --max-time 30 -H 'content-type: application/json' \
    -d "$(jq -cn --arg model "$model" '{model: $model, keep_alive: 0}')" \
    "$OLLAMA_URL/api/generate" >/dev/null
  echo "unloading $model (keep_alive: 0)"
done <<<"$models"

for ((s = 0; ; s++)); do
  # An Ollama that stopped answering holds no model either.
  models=$(loaded | paste -sd ' ') || true
  [ -z "$models" ] && break
  if [ "$s" -ge "$OLLAMA_UNLOAD_TIMEOUT_S" ]; then
    echo "ollama: $models still loaded $OLLAMA_UNLOAD_TIMEOUT_S s after keep_alive: 0" >&2
    exit 1
  fi
  sleep 1
done
echo "ollama has no model loaded; its server keeps running"
