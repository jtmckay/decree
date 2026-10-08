#!/usr/bin/env bash
# develop's verify: {ai_title} checks the acceptance criteria and ends its reply
# with `VERDICT: pass` or `VERDICT: fail`; the last such line is the event. A
# reply without one fails the attempt, so decree asks again. Like implement, it
# writes STOP instead of guessing.
set -euo pipefail
. "${DECREE_LIB}/ai.sh"

progress="${DECREE_RUN_DIR}/progress.md"
stop="${DECREE_RUN_DIR}/STOP"
stopped() {
  [ -f "${stop}" ] || return 1
  cat "${stop}" >&2
  echo stop > "${DECREE_EVENT_FILE}"
}
stopped && exit 0

prompt="Read ${DECREE_MESSAGE}. Verify that all requirements and
acceptance criteria are met; ${progress} notes what was done. Run the tests,
and report what passes and what fails. Change no code. If a criterion needs a
decision the message does not make, write the question to ${stop} and stop.
End your reply with one line: VERDICT: pass if everything passes, else
VERDICT: fail."
echo "=== AI prompt (verification) ==="
echo "${prompt}"
reply=$(mktemp)
trap 'rm -f "${reply}"' EXIT
ai "${prompt}" | tee "${reply}"
stopped && exit 0

# The last line naming a verdict, Markdown emphasis and backticks allowed.
verdict=$(tr -d '*`' < "${reply}" \
  | sed -nE 's/^[[:space:]]*VERDICT:[[:space:]]*(pass|fail)[[:space:]]*$/\1/p' | tail -n 1)
if [ -z "${verdict}" ]; then
  echo "verify: no 'VERDICT: pass' or 'VERDICT: fail' line in the reply" >&2
  exit 1
fi
echo "${verdict}" > "${DECREE_EVENT_FILE}"
