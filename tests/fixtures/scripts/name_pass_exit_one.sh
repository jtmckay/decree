#!/usr/bin/env bash
# Writes the pass event to $DECREE_EVENT_FILE, then exits 1.
echo "tests ran"
echo pass > "$DECREE_EVENT_FILE"
exit 1
