#!/usr/bin/env bash
# develop's implement, a stand-in for an AI agent: scripts/develop/ is checked
# before scripts/. The machine's attempt list says which model does this
# attempt, in $DECREE_ATTEMPT_VALUE: a choice of how, not of what runs next.
# A non-zero exit is error; decree then runs the next attempt in place.
set -euo pipefail
prompt="Read $DECREE_MESSAGE and make the change it asks for."
case "$DECREE_ATTEMPT_VALUE" in
  claude) claude --permission-mode auto -p "$prompt" ;;
  *)      ollama run qwen3:8b "$prompt" ;;
esac
