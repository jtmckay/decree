# tmux_service.sh: sourced by the use_* scripts, never run on its own, so no
# machine names it and it is not executable. A service the scripts start lives
# in a tmux session named after it: a person watches it with
# `tmux attach -t <session>`. decree runs the scripts themselves directly,
# never inside tmux.

# answers <health url>: whether the service answers there, whoever runs it.
answers() {
  curl -fsS --max-time 2 "$1" >/dev/null 2>&1
}

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

# wait_until_up <health url> <seconds>: poll the URL once a second until it answers.
wait_until_up() {
  local url=$1 seconds=$2 _
  for _ in $(seq "$seconds"); do
    if answers "$url"; then
      echo "$url answers"
      return 0
    fi
    sleep 1
  done
  echo "$url did not answer within $seconds s" >&2
  return 1
}
