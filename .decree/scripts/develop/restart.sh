#!/usr/bin/env bash
# onentry script of `done`, after commit: hold the queue once this run has
# committed, so `decree process` exits and .decree/scripts/develop/run-queue.sh
# builds decree from the new commit before the next migration runs. Each
# migration here changes decree itself, so the next one must run on the result.
# Only migrations: an inbox run is not part of the queue's rebuild loop.
set -euo pipefail
[ "${DECREE_TRIGGER:-}" = migration ] || exit 0
"${DECREE}" stop -m "develop committed ${DECREE_MESSAGE_ID}: run-queue.sh rebuilds decree and goes on"
