#!/usr/bin/env bash
# Fails unless its attempt's value is claude; prints its attempt variables.
echo "attempt $DECREE_ATTEMPT of $DECREE_MAX_ATTEMPTS: ${DECREE_ATTEMPT_VALUE-unset} in ${DECREE_ATTEMPT_VALUES-unset}"
[ "$DECREE_ATTEMPT_VALUE" = claude ]
