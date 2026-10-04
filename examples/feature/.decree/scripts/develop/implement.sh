#!/usr/bin/env bash
# develop's own implement: scripts/develop/ is checked before scripts/.
# timeout_s in the machine bounds it; decree kills the
# whole process group if it runs over.
set -euo pipefail
claude --permission-mode auto -p "Read $DECREE_MESSAGE and make the change it asks for."
