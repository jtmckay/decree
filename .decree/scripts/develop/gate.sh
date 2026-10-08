#!/usr/bin/env bash
# develop's gate: the project's own checks, also kept in gate.log for fix. It
# names `pass` if they all pass and `fail` if one fails; a non-zero exit, such
# as a missing tool, means they could not run, and ends the run without fix.
# This repository's checks: cargo fmt, clippy and the tests, one per line
# (tests/ci_test.rs reads them to keep .github/workflows/ci.yml the same).
set -euo pipefail
log="${DECREE_RUN_DIR}/gate.log"

# A tool the checks need is not on PATH: they cannot run.
missing() {
  echo "gate: $1 not found on PATH; the checks cannot run" | tee "${log}" >&2
  exit 1
}
command -v cargo >/dev/null || missing cargo

# The checks, as one && chain: the first that fails names `fail`.
checks() {
  cargo fmt --check &&
    cargo clippy --all-targets -- -D warnings &&
    cargo test
}

if checks 2>&1 | tee "${log}"; then
  echo pass > "${DECREE_EVENT_FILE}"
else
  echo fail > "${DECREE_EVENT_FILE}"
fi
