#!/usr/bin/env bash
set -euo pipefail

# Default DECREE_CONTAINER to hostname (Docker sets this to 12-char container ID)
DECREE_CONTAINER="${DECREE_CONTAINER:-$HOSTNAME}"
export DECREE_CONTAINER

# Validate DECREE_CONTAINER: only [a-zA-Z0-9_-], no __, no empty
if [[ -z "$DECREE_CONTAINER" ]]; then
  echo "ERROR: DECREE_CONTAINER must not be empty" >&2
  exit 1
fi
if [[ "$DECREE_CONTAINER" == *"__"* ]]; then
  echo "ERROR: DECREE_CONTAINER must not contain '__': $DECREE_CONTAINER" >&2
  exit 1
fi
if ! [[ "$DECREE_CONTAINER" =~ ^[a-zA-Z0-9_-]+$ ]]; then
  echo "ERROR: DECREE_CONTAINER contains invalid characters (only [a-zA-Z0-9_-] allowed): $DECREE_CONTAINER" >&2
  exit 1
fi

# Initialize decree if .decree/ doesn't exist
if [[ ! -d /work/.decree ]]; then
  decree init --no-color </dev/null
fi

# Run routine sync if /routines contains any .sh files
if compgen -G "/routines/*.sh" >/dev/null 2>&1; then
  routine-sync.sh
fi

# If CMD arguments were passed, exec them directly
if [[ $# -gt 0 ]]; then
  exec "$@"
fi

# Default behavior: daemon or interactive shell
DECREE_DAEMON="${DECREE_DAEMON:-true}"
if [[ "$DECREE_DAEMON" == "true" ]]; then
  exec decree daemon --no-color --interval "${DECREE_INTERVAL:-2}"
else
  exec bash
fi
