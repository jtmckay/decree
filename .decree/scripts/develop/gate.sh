#!/usr/bin/env bash
# develop's gate: formatting, lints and tests, also kept in gate.log for fix. A
# non-zero exit sends the run to fix.
set -euo pipefail
{
  cargo fmt --check &&
    cargo clippy --all-targets -- -D warnings &&
    cargo test
} 2>&1 | tee "${DECREE_RUN_DIR}/gate.log"
