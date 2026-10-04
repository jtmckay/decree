# tmux_service.sh: sourced by the use_* scripts, never run on its own, so no
# machine names it and it is not executable. Each service lives in a tmux
# session named after it: a person watches it with `tmux attach -t <session>`.
# decree runs the scripts themselves directly, never inside tmux.

# How long end_session waits for a service to stop answering.
TMUX_END_TIMEOUT_S="${TMUX_END_TIMEOUT_S:-30}"

# ensure_session <session> <command>: use the tmux session if it is running, or
# start <command> in a new detached one.
ensure_session() {
  local session=$1 command=$2
  if tmux has-session -t "=$session" 2>/dev/null; then
    echo "using the running tmux session $session"
  else
    tmux new-session -d -s "$session" "$command"
    echo "started \`$command\` in a new tmux session $session"
  fi
  echo "watch it with: tmux attach -t $session"
}

# end_session <session> <health url>: end the tmux session if it is running,
# then wait until the service stops answering. A service that still answers
# runs outside tmux.
end_session() {
  local session=$1 url=$2 _
  if tmux has-session -t "=$session" 2>/dev/null; then
    tmux kill-session -t "=$session"
    echo "ended the tmux session $session"
  fi
  for _ in $(seq "$TMUX_END_TIMEOUT_S"); do
    curl -fsS --max-time 2 "$url" >/dev/null 2>&1 || return 0
    sleep 1
  done
  echo "$session still answers at $url ${TMUX_END_TIMEOUT_S} s after its tmux session ended," \
    "so it is running outside tmux (for example as a systemd service)." \
    "Stop it, for example with \`sudo systemctl stop $session\`, and keep it stopped with" \
    "\`sudo systemctl disable $session\`, or stop the process that listens there." >&2
  return 1
}

# wait_until_up <health url> <seconds>: poll the URL once a second until it answers.
wait_until_up() {
  local url=$1 seconds=$2 _
  for _ in $(seq "$seconds"); do
    if curl -fsS --max-time 2 "$url" >/dev/null 2>&1; then
      echo "$url answers"
      return 0
    fi
    sleep 1
  done
  echo "$url did not answer within $seconds s" >&2
  return 1
}
