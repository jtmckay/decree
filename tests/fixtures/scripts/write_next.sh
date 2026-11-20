#!/usr/bin/env bash
# Writes next.md in the run's folder: a message for `step_child`, with params and a key of its
# own, for a `machine: { message: next.md }` invoke.
printf -- '---\nmachine: step_child\nparams:\n  label: from-file\norder: develop/A01-x.md\n---\n# From the file\n' > "$DECREE_RUN_DIR/next.md"
