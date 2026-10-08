#!/usr/bin/env bash
# Names stop (with a question in STOP) unless its attempt's value is claude; prints its attempt.
echo "attempt $DECREE_ATTEMPT of $DECREE_MAX_ATTEMPTS: ${DECREE_ATTEMPT_VALUE-unset}"
[ "$DECREE_ATTEMPT_VALUE" = claude ] && exit 0
echo "Which limit?" > "$DECREE_RUN_DIR/STOP"
echo stop > "$DECREE_EVENT_FILE"
