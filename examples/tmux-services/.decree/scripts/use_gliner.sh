#!/usr/bin/env bash
# onentry (illustrated_post's root): use GLiNER2.5-Decide if it already
# answers, whoever runs it; otherwise use the gliner tmux session, or start the
# server in a new one, and wait until it answers. It runs on CPU and stays up.
set -euo pipefail
GLINER_SESSION="${GLINER_SESSION:-gliner}"
# The one copy of the server, in examples/route-by-complexity/; set this when the
# example is copied out of the repository.
GLINER_SERVER="${GLINER_SERVER:-$DECREE_PROJECT_ROOT/../route-by-complexity/gliner/decide_server.py}"
GLINER_PYTHON="${GLINER_PYTHON:-python3}"   # or: uv run --with 'gliner2[local,train]' python
GLINER_HEALTH="${GLINER_HEALTH:-http://127.0.0.1:8090/health}"
GLINER_START_TIMEOUT_S="${GLINER_START_TIMEOUT_S:-600}"   # the first start downloads about 4.8 GB
source "$(dirname "${BASH_SOURCE[0]}")/tmux_service.sh"

if answers "$GLINER_HEALTH"; then
  echo "gliner already answers at $GLINER_HEALTH: using it, whoever runs it"
  exit 0
fi
if [ ! -f "$GLINER_SERVER" ]; then
  echo "gliner: no server at $GLINER_SERVER; set GLINER_SERVER to decide_server.py" >&2
  exit 1
fi
ensure_session "$GLINER_SESSION" "$GLINER_PYTHON $(printf '%q' "$GLINER_SERVER")"
wait_until_up "$GLINER_HEALTH" "$GLINER_START_TIMEOUT_S" || {
  echo "gliner did not start; see why with: tmux attach -t $GLINER_SESSION" >&2
  exit 1
}
