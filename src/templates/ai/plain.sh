# Ask {ai_title}; the prompt is the only argument. The agent names no event (it
# writes STOP instead), so a script it runs, such as the gate, cannot name the
# caller's.
ai() {
  local -x DECREE_EVENT_FILE=/dev/null
  {ai_cli} "$1"
}
