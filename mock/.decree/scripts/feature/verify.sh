#!/usr/bin/env bash
# feature's verify: run the tests and say what happened. A script names its
# own event by printing it as JSON on the last line: pass or fail.
set -uo pipefail
cargo test 2>&1 | tail -n 40
if [ "${PIPESTATUS[0]}" -eq 0 ]; then
  echo '{"event":"pass"}'
else
  echo '{"event":"fail"}'
fi
