#!/usr/bin/env bash
# feature-only invoke of verify (a router state). A passing test run is decided here,
# deterministically. Only failures reach the LLM, which then picks
# retry, split or fail from the options the machine allows.
set -uo pipefail
cargo test 2>&1 | tail -n 40
if [ "${PIPESTATUS[0]}" -eq 0 ]; then
  echo '{"event":"pass"}'
fi
exit 0
