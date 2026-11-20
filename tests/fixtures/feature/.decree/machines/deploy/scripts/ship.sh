#!/usr/bin/env bash
# Runs only after a person replied `approve`. Their note, if any, is in
# $DECREE_RECEIVED.
set -euo pipefail
[ -n "${DECREE_RECEIVED:-}" ] && sed '1,/^---$/d; 1,/^---$/d' "$DECREE_RECEIVED" || true
./scripts/release.sh
