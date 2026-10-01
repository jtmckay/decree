#!/usr/bin/env bash
# onexit script of implement: keep a diff of what this round changed.
# A failing onexit script is recorded in events.jsonl and otherwise ignored.
set -euo pipefail
git diff "$(cat "$DECREE_RUN_DIR/baseline")" > "$DECREE_RUN_DIR/round-$DECREE_VISITS.diff"
echo "saved round-$DECREE_VISITS.diff"
