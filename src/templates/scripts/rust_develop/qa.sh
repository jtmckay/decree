#!/usr/bin/env bash
# rust_develop's qa: {ai_title} fixes what the gate reported. Like implement,
# it writes STOP instead of guessing.
set -euo pipefail

{ai_function}

progress="${DECREE_RUN_DIR}/progress.md"
stop="${DECREE_RUN_DIR}/STOP"
stopped() {
  [ -f "${stop}" ] || return 1
  cat "${stop}" >&2
  echo stop > "${DECREE_EVENT_FILE}"
}
stopped && exit 0

prompt="Read ${DECREE_MESSAGE}. The gate (cargo fmt --check, cargo clippy
--all-targets -- -D warnings, cargo test) failed; its output is in
${DECREE_RUN_DIR}/gate.log, and ${progress} notes what was done so far.
Fix the failures, append a line to ${progress} for each fix, and run the
gate again. If a failure needs a decision the message does not make, write
the question to ${stop} and stop."
echo "=== AI prompt (QA) ==="
echo "${prompt}"
ai "${prompt}"
stopped || true
