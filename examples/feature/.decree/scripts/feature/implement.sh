#!/usr/bin/env bash
# feature's own implement: scripts/feature/ is checked before scripts/.
# The agent does the work. The exit code is the only
# signal; the machine decides what happens next. (The real built-in script
# also waits out Claude usage limits and resumes the session: see the develop
# machines `decree init` writes.)
set -euo pipefail
claude --permission-mode auto -p "Read $DECREE_MESSAGE and implement it.
This is round $DECREE_VISITS, attempt $DECREE_ATTEMPT of $DECREE_MAX_ATTEMPTS.
Logs and diffs from earlier rounds are in $DECREE_RUN_DIR."
