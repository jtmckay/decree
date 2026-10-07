#!/usr/bin/env bash
# develop_by_size's implement: one attempt. $DECREE_ATTEMPT_VALUE names the
# tool, then the tests run: an attempt succeeds when the change passes them, so
# a failing attempt exits non-zero and decree runs the next value.
#   local   a model served by Ollama, through OpenCode. `opencode run --model
#           <provider>/<model>` runs one prompt without the TUI
#           (https://opencode.ai/docs/cli/); the `ollama` provider and this
#           model must be in opencode.json, as the example's README shows
#           (https://opencode.ai/docs/providers/).
#   claude  claude -p, in small steps noted in progress.md, so a later attempt
#           continues where this one stopped. If the message is unclear, it
#           writes the question to STOP instead of guessing; the state's event
#           is then `stop`, and the run fails until a person answers it,
#           deletes STOP and runs `decree process --retry`.
set -euo pipefail
OLLAMA_MODEL="${OLLAMA_MODEL:-qwen3-coder:30b}"
TEST_CMD="${TEST_CMD:-cargo test}"
progress="${DECREE_RUN_DIR}/progress.md"
stop="${DECREE_RUN_DIR}/STOP"
stopped() {
  [ -f "${stop}" ] || return 1
  cat "${stop}" >&2
  echo stop > "${DECREE_EVENT_FILE}"
}
stopped && exit 0

case "${DECREE_ATTEMPT_VALUE:-}" in
  local)
    prompt="Read ${DECREE_MESSAGE} and implement it. It is a small, mechanical change:
change only what it asks for, and keep the tests passing."
    echo "=== opencode run --model ollama/${OLLAMA_MODEL} ==="
    echo "${prompt}"
    opencode run --model "ollama/${OLLAMA_MODEL}" "${prompt}"
    ;;
  claude)
    prompt="Read ${DECREE_MESSAGE} and implement all requirements with proper error
handling and tests. Work in small steps, one requirement at a time, and keep the
code compiling between steps. After each step, append a line to ${progress}:
what is done and what is next. If ${progress} exists, an earlier attempt was
cut short: check the code against it and continue from there.
If the message is ambiguous or conflicts with the code, do not guess: write
the question to ${stop} and stop.
Logs of earlier attempts, a local model's included, are in ${DECREE_RUN_DIR}."
    echo "=== claude -p ==="
    echo "${prompt}"
    claude -p "${prompt}"
    stopped && exit 0
    ;;
  *)
    echo "implement: unknown attempt value '${DECREE_ATTEMPT_VALUE:-}'; use local or claude" >&2
    exit 2
    ;;
esac

echo "=== ${TEST_CMD} ==="
${TEST_CMD}
echo "tests pass with ${DECREE_ATTEMPT_VALUE}"
