# Ask {ai_title}; the prompt is the only argument. The agent names no event (it
# writes STOP instead): a script it runs, such as the gate, gets
# DECREE_EVENT_FILE=/dev/null, and the caller's event file is emptied after the
# call, in case the agent found its path anyway.
ai() {
  local caller_event=${DECREE_EVENT_FILE:-} status=0
  local -x DECREE_EVENT_FILE=/dev/null
  {ai_cli} "$1" || status=$?
  [ -z "${caller_event}" ] || : > "${caller_event}"
  return "${status}"
}
