#!/usr/bin/env bash
# Writes the pass event to $DECREE_EVENT_FILE with surrounding whitespace.
printf '  \t pass \n\n' > "$DECREE_EVENT_FILE"
