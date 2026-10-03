#!/usr/bin/env bash
# rust_develop's qa: {ai_title} fixes what build or test reported. Its exit
# code is the result.
set -euo pipefail

{ai_function}

prompt="Read ${DECREE_MESSAGE}, build output at ${DECREE_RUN_DIR}/build.log,
test output at ${DECREE_RUN_DIR}/test-output.log. Fix any failures. Run cargo
build --release and cargo test again. Exit 0 only if everything passes."
echo "=== AI prompt (QA) ==="
echo "${prompt}"
ai "${prompt}"
