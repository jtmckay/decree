#!/usr/bin/env bash
# Copies the run's .running to running.copy, and records its own pid in self.pid.
cp "$DECREE_RUN_DIR/.running" "$DECREE_RUN_DIR/running.copy"
echo "$$" > "$DECREE_RUN_DIR/self.pid"
