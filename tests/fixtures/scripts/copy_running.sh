#!/usr/bin/env bash
# Copies the run's .running to running.copy, and records its own pid in self.pid.
# decree writes .running right after spawning this script, so wait for it (up to 10 s).
for _ in $(seq 1000); do
  [ -e "$DECREE_RUN_DIR/.running" ] && break
  sleep 0.01
done
cp "$DECREE_RUN_DIR/.running" "$DECREE_RUN_DIR/running.copy"
echo "$$" > "$DECREE_RUN_DIR/self.pid"
