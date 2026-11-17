---
name: deploy
machine:
  description: Build, ask a person to approve, then ship.
  initial: build
  states:
    build:
      invoke: build
      transitions:
        done: approval
    approval:
      invoke:
        reply:
          question: Ship this build?
          notify: notify_person
          timeout: 1d
      transitions:
        approve:
          target: ship
          description: Ship this build.
        reject:
          target: rejected
          description: Do not ship.
    ship:
      invoke: ship
      transitions:
        done: done
    done:
      final: true
    rejected:
      final: true
    failed:
      final: true
scripts:
  notify_person.sh: |
    #!/usr/bin/env bash
    # The notify script of deploy's approval, shared by any reply state that asks a
    # person. decree only knows the run is waiting for one of the options; how a
    # person is asked is up to this script: chat, email, an issue, or a UI that
    # writes the reply message itself. Here it prints the commands; a real one would post them.
    set -euo pipefail
    title=$(grep -m1 '^# ' "$DECREE_MESSAGE" | sed 's/^# //' || true)
    echo "Waiting for a person: $DECREE_MACHINE/$DECREE_STATE ${title:+- $title}"
    echo "Question: $DECREE_QUESTION"
    echo "Options (event: description):"
    cat "$DECREE_CHOICES"
    echo
    for event in $DECREE_EVENTS; do
      [ "$event" = error ] && continue
      echo "  decree event $DECREE_WAIT_ID $event -m \"<note>\""
    done
  build.sh: |
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build --release
  ship.sh: |
    #!/usr/bin/env bash
    # Runs only after a person replied `approve`. Their note, if any, is in
    # $DECREE_RECEIVED.
    set -euo pipefail
    [ -n "${DECREE_RECEIVED:-}" ] && sed '1,/^---$/d; 1,/^---$/d' "$DECREE_RECEIVED" || true
    ./scripts/release.sh
---
# deploy

Build, ask a person to approve, then ship.
