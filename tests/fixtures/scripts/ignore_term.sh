#!/usr/bin/env bash
# Like sleep_long, but the script and its child ignore SIGTERM.
trap "" TERM
sleep 100 &
echo "$!" > "$DECREE_RUN_DIR/child.pid"
wait
