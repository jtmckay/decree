---
name: hello
machine:
  description: Run one script.
  initial: greet
  states:
    greet:
      invoke: greet
      transitions:
        done: done
    done:
      final: true
    failed:
      final: true
scripts:
  greet.sh: |
    #!/usr/bin/env bash
    # hello's only script. Exit 0 is the done event; anything else is error.
    set -euo pipefail
    echo "hello from $DECREE_MACHINE, run $DECREE_MESSAGE_ID"
---
# hello

Run one script.
