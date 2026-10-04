#!/usr/bin/env bash
# Root onentry script. Runs once when the run starts, and again when
# `decree process --retry` continues the run, so it must be safe to repeat: the
# baseline is written only the first time.
set -euo pipefail
base="$DECREE_RUN_DIR/baseline"
[ -f "$base" ] || git rev-parse HEAD > "$base"
echo "baseline $(cat "$base")"
