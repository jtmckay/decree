#!/usr/bin/env bash
# develop's implement, a stand-in for an AI agent: scripts/develop/ is checked
# before scripts/. The machine's attempt list says which model does this
# attempt, in $DECREE_ATTEMPT_VALUE: a choice of how, not of what runs next.
# `local` is a model served by Ollama through OpenCode; `claude` is Claude. Both go
# through lib/ai.sh's `ai`, which DECREE_AI points at a backend.
# A non-zero exit is error; decree then runs the next attempt in place.
set -euo pipefail
. "${DECREE_LIB}/ai.sh"
prompt="Read $DECREE_MESSAGE and make the change it asks for."
case "$DECREE_ATTEMPT_VALUE" in
  claude) DECREE_AI=claude ai "$prompt" ;;
  *)      DECREE_AI=opencode OPENCODE_MODEL="${OPENCODE_MODEL:-ollama/qwen3:8b}" ai "$prompt" ;;
esac
