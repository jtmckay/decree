#!/usr/bin/env bash
# Hand the message to opencode to implement.
set -euo pipefail
opencode run "Read ${DECREE_MESSAGE} and implement the requirements.
Previous attempt logs (if any) are in ${DECREE_RUN_DIR} for context."
