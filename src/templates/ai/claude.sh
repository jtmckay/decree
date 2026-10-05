# Ask Claude; the prompt is the only argument. Each call is a new session,
# listed in the run's sessions.txt with its transcript, which Claude writes as
# it goes and keeps even if the session dies.
# When Claude stops at its usage limit (its output names a "usage limit" and a
# "reset"), wait until the reset time it names, or an hour if it names none,
# then resume the same session with the same prompt.
# CLAUDE_PERMISSION_MODE is passed to every call, so the run does not inherit your
# interactive default (in `plan` mode Claude can only plan, and changes nothing). `auto`
# approves what Claude's classifier judges safe; `acceptEdits` approves only file edits.
CLAUDE_PERMISSION_MODE="${CLAUDE_PERMISSION_MODE:-auto}"
ai() {
  local prompt=$1 session out status
  session=$(new_session_id)
  echo "=== claude session ${session} ===" >&2
  echo "${DECREE_STATE} ${session} ${HOME}/.claude/projects/$(pwd | sed 's/[^A-Za-z0-9]/-/g')/${session}.jsonl" \
    >> "${DECREE_RUN_DIR}/sessions.txt"
  local session_flag=(--session-id "${session}")
  while true; do
    out=$(mktemp)
    # stdout to stdout and stderr to stderr, both also kept in $out
    if { claude -p --permission-mode "${CLAUDE_PERMISSION_MODE}" "${session_flag[@]}" "${prompt}" 2>&1 1>&3 3>&- | tee -a "${out}" >&2; } 3>&1 | tee -a "${out}"; then
      status=0
    else
      status=$?
    fi
    if [ "${status}" -eq 0 ] || ! usage_limit "${out}"; then
      rm -f "${out}"
      return "${status}"
    fi
    wait_for_reset "${out}"
    rm -f "${out}"
    echo "[Claude token limit] Resuming session ${session}" >&2
    session_flag=(--resume "${session}")
  done
}

# A random version 4 UUID, the form `claude --session-id` takes.
new_session_id() {
  printf '%04x%04x-%04x-4%03x-%04x-%04x%04x%04x\n' "${RANDOM}" "${RANDOM}" "${RANDOM}" \
    $((RANDOM & 0xfff)) $((RANDOM & 0x3fff | 0x8000)) "${RANDOM}" "${RANDOM}" "${RANDOM}"
}

# Claude's wording varies: "usage limit reached … reset at 10:00 PM", "You've hit
# your session limit · resets 1:10am (America/Denver)", weekly and 5-hour limits.
usage_limit() {
  grep -qiE '(usage|session|weekly|daily|5-hour) limit|limit (reached|hit)' "$1" && grep -qi 'reset' "$1"
}

# Sleep until the time in "Limits reset at 10:00 PM" (local time, tomorrow if
# it has passed today), or for an hour if there is no such time.
wait_for_reset() {
  local at h m now_h now_m now_s now wait=3600
  # "reset at 10:00 pm", "resets 1:10am", "resets 3am": read as local time.
  at=$(tr '[:upper:]' '[:lower:]' < "$1" | grep -oE 'resets?( +at)? +[0-9]{1,2}(:[0-9]{2})? *[ap]m' \
    | head -n 1 | grep -oE '[0-9]{1,2}(:[0-9]{2})? *[ap]m') || true
  IFS=: read -r now_h now_m now_s <<< "$(date +%H:%M:%S)"
  now=$((10#${now_h} * 3600 + 10#${now_m} * 60 + 10#${now_s}))
  if [ -n "${at}" ]; then
    h=${at%%[!0-9]*}
    h=$((10#${h}))
    m=0
    case ${at} in *:*) m=${at#*:}; m=$((10#${m:0:2})) ;; esac
    if [ "${h}" -le 12 ] && [ "${m}" -le 59 ]; then
      h=$((h % 12))
      case ${at} in *pm) h=$((h + 12)) ;; esac
      wait=$((h * 3600 + m * 60 - now))
      [ "${wait}" -gt 0 ] || wait=$((wait + 86400))
    fi
  fi
  local reset_at=$(((now + wait) % 86400))
  printf '[Claude token limit] Usage limit reached. Waiting until %02d:%02d (%dm %ds) to retry.\n' \
    $((reset_at / 3600)) $((reset_at % 3600 / 60)) $((wait / 60)) $((wait % 60)) >&2
  sleep "${wait}"
}
