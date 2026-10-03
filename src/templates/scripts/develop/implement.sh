#!/usr/bin/env bash
# develop's implement: {ai_title} implements the message.
set -euo pipefail

{ai_function}

prompt="Read ${DECREE_MESSAGE} and implement all requirements.
Previous attempt logs (if any) are in ${DECREE_RUN_DIR} for context.
Follow best practices: clean code, proper error handling, and tests
where appropriate."
echo "=== AI prompt (implementation) ==="
echo "${prompt}"
ai "${prompt}"
