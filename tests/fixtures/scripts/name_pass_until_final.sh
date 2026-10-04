#!/usr/bin/env bash
# Logs what $DECREE_EVENT_FILE holds when it starts, writes pass to it, and fails on
# every attempt but the final one.
echo "attempt $DECREE_ATTEMPT, event file holds [$(cat "$DECREE_EVENT_FILE")]"
echo pass > "$DECREE_EVENT_FILE"
[ "$DECREE_FINAL_ATTEMPT" = true ]
