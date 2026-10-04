#!/usr/bin/env bash
# Release build, also kept in build.log for qa.
set -euo pipefail
echo "=== Building (release) ==="
cargo build --release 2>&1 | tee "${DECREE_RUN_DIR}/build.log"
