#!/usr/bin/env bash
# develop_by_size's verify: the project's tests. Exit 0 is done; anything else
# is error, which escalates a local attempt to Claude once.
set -euo pipefail
TEST_CMD="${TEST_CMD:-cargo test}"
echo "=== ${TEST_CMD} ==="
${TEST_CMD}
echo "tests pass"
