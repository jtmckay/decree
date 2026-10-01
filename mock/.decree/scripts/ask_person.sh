#!/usr/bin/env bash
# Shared onentry script of waiting states (feature's review, deploy's approval).
# decree only knows the run is waiting for one of $DECREE_ACCEPTS. How a person
# is asked is up to this script: chat, email, an issue, or a UI that writes the
# reply message itself. Here it prints the commands; a real one would post them.
set -euo pipefail
title=$(grep -m1 '^# ' "$DECREE_MESSAGE" | sed 's/^# //' || true)
echo "Waiting for a person: $DECREE_MACHINE/$DECREE_STATE ${title:+- $title}"
echo "Reply with one of:"
for event in $DECREE_ACCEPTS; do
  echo "  decree event $DECREE_WAIT_ID $event -m \"<note>\""
done
