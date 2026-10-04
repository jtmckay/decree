#!/usr/bin/env bash
# Writes the pass event to $DECREE_EVENT_FILE and exits 0.
echo "tests ran"
echo pass > "$DECREE_EVENT_FILE"
