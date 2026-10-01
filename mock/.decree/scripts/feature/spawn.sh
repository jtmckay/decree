#!/usr/bin/env bash
# Invoke of spawn_followups. Only `decree emit` writes to the inbox, and only
# for machines listed in this state's `emits`. decree sets parent and depth.
set -euo pipefail
parts="$DECREE_RUN_DIR/parts"
mkdir -p "$parts"
claude --permission-mode auto -p "Read $DECREE_MESSAGE. Split it into smaller specs, one markdown file each, in $parts."
for spec in "$parts"/*.md; do
  decree emit --machine feature < "$spec"
done
