# The AI helper the built-in machines' scripts source: `. "$DECREE_LIB/ai.sh"`,
# then `ai "<prompt>"` asks {ai_title} and prints its reply.
#
# Another backend, such as a local model, is one more function, chosen by a
# variable. With `attempts: [local, claude]` on a script invoke, decree runs the
# script once per entry until one succeeds, with the entry in
# DECREE_ATTEMPT_VALUE; add at the end of this file:
#
#   ai_local() { opencode run --model ollama/<model> "$1"; }
#   case "${DECREE_ATTEMPT_VALUE:-}" in
#     local) ai() { ai_local "$1"; } ;;
#   esac
