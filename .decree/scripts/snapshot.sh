#!/usr/bin/env bash
# onentry script of implement: runs every time the state is entered (each
# round), but not on a mechanical retry (attempt) inside the same visit.
set -euo pipefail
ref=$(git stash create || true)
if [ -n "$ref" ]; then
  git stash store -m "decree $DECREE_MESSAGE_ID round $DECREE_VISITS" "$ref"
  echo "snapshot $ref"
else
  echo "clean tree, no snapshot"
fi
