#!/usr/bin/env bash
# develop's fix: Claude fixes what the gate reported. Like implement, it
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

prompt="Read ${DECREE_MESSAGE}. The project's gate
(.decree/scripts/develop/gate.sh) failed; its output is in
${DECREE_RUN_DIR}/gate.log, and ${progress} notes what was done so far.
Fix the failures, append a line to ${progress} for each fix, and run the
gate again. If a failure needs a decision the message does not make, write
the question to ${stop} and stop."
echo "=== AI prompt (fix) ==="
echo "${prompt}"
ai "${prompt}"
stopped || true
