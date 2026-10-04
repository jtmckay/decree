#!/usr/bin/env bash
# The gate: formatting, lints and tests, also kept in gate.log for qa.
set -euo pipefail
{
  cargo fmt --check &&
    cargo clippy --all-targets -- -D warnings &&
    cargo test
} 2>&1 | tee "${DECREE_RUN_DIR}/gate.log"
