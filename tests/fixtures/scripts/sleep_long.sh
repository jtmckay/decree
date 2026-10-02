#!/usr/bin/env bash
# Starts sleep 100, records its pid in the run folder, and waits for it.
sleep 100 &
echo "$!" > "$DECREE_RUN_DIR/child.pid"
wait
