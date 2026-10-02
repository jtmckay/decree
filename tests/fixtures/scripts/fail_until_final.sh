#!/usr/bin/env bash
# Fails on every attempt but the final one.
echo "attempt $DECREE_ATTEMPT of $DECREE_MAX_ATTEMPTS"
[ "$DECREE_FINAL_ATTEMPT" = true ]
