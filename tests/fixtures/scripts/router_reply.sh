#!/usr/bin/env bash
# A router script: replies with reply.json from the project root, as written.
set -euo pipefail
cp "$DECREE_PROJECT_ROOT/reply.json" "$DECREE_REPLY"
