#!/usr/bin/env bash
# Shared by every machine: feature uses it as its root onexit script, develop
# as the onentry script of failed, so it runs once when a run fails. It reports the state
# the run ended in, from the last transition event.
set -euo pipefail
state=$(grep '"type":"transition"' "$DECREE_RUN_DIR/events.jsonl" | tail -n 1 | sed 's/.*"to":"\([^"]*\)".*/\1/')
echo "$DECREE_MACHINE $DECREE_MESSAGE_ID finished: $state"
