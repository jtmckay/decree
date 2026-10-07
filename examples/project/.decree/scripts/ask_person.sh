#!/usr/bin/env bash
# The ask script of deploy's approval, shared by any person state. decree only
# knows the run is waiting for one of the options; how a person is asked is up
# to this script: chat, email, an issue, or a UI that writes the reply message
# itself. Here it prints the commands; a real one would post them.
set -euo pipefail
title=$(grep -m1 '^# ' "$DECREE_MESSAGE" | sed 's/^# //' || true)
echo "Waiting for a person: $DECREE_MACHINE/$DECREE_STATE ${title:+- $title}"
echo "Question: $DECREE_QUESTION"
echo "Options (event: description):"
cat "$DECREE_CHOICES"
echo
for event in $DECREE_EVENTS; do
  [ "$event" = error ] && continue
  echo "  decree event $DECREE_WAIT_ID $event -m \"<note>\""
done
