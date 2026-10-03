#!/usr/bin/env bash
# Tests, also kept in test-output.log for qa.
set -euo pipefail
echo "=== Running tests ==="
cargo test 2>&1 | tee "${DECREE_RUN_DIR}/test-output.log"
