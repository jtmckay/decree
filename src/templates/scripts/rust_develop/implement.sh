#!/usr/bin/env bash
# rust_develop's implement: {ai_title} implements the message in small steps,
# noting each in progress.md, so a retry continues where the last attempt
# stopped. If the message is unclear, it writes the question to STOP instead of
# guessing; the run fails until a person answers it, deletes STOP and runs
# `decree retry`.
set -euo pipefail

{ai_function}

progress="${DECREE_RUN_DIR}/progress.md"
stop="${DECREE_RUN_DIR}/STOP"
stopped() {
  [ -f "${stop}" ] || return 1
  cat "${stop}" >&2
  echo '{"event": "stop"}'
}
stopped && exit 0

prompt="You are a senior Rust engineer. Read ${DECREE_MESSAGE} and
implement all requirements with proper error handling and tests.
Work in small steps, one requirement at a time, and keep the code compiling
between steps. After each step, append a line to ${progress}: what is done
and what is next. If ${progress} exists, an earlier attempt was cut short:
check the code against it and continue from there.
If the message is ambiguous or conflicts with the code, do not guess: write
the question to ${stop} and stop.
Logs of earlier attempts (if any) are in ${DECREE_RUN_DIR}."
echo "=== AI prompt (implementation) ==="
echo "${prompt}"
ai "${prompt}"
stopped || true
