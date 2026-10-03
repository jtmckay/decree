#!/usr/bin/env bash
# develop's verify: {ai_title} checks the requirements and acceptance criteria.
# Its exit code is the result.
set -euo pipefail

{ai_function}

prompt="Read ${DECREE_MESSAGE}. Verify that all requirements and
acceptance criteria are met. Run any tests. Report what passes and what
fails. Exit 0 if everything passes, exit 1 if anything fails."
echo "=== AI prompt (verification) ==="
echo "${prompt}"
ai "${prompt}"
