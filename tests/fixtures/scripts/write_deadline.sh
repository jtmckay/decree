#!/usr/bin/env bash
# Writes deadline.json in the run's folder, `expires` an instant in 2099, and deadline.txt,
# the same instant alone, for a `reply` invoke's `timeout_at`.
printf '{"lease": {"expires": "2099-01-02T03:04:05.678Z"}}\n' > "$DECREE_RUN_DIR/deadline.json"
printf '2099-01-02T03:04:05.678Z\n' > "$DECREE_RUN_DIR/deadline.txt"
