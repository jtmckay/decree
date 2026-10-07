#!/usr/bin/env bash
# feature's verify: run the tests and say what happened. A script names its
# own event by writing it to $DECREE_EVENT_FILE: pass or fail.
set -uo pipefail
cargo test 2>&1 | tail -n 40
if [ "${PIPESTATUS[0]}" -eq 0 ]; then
  echo pass > "$DECREE_EVENT_FILE"
else
  echo fail > "$DECREE_EVENT_FILE"
fi
