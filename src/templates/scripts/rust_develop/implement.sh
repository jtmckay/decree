#!/usr/bin/env bash
# rust_develop's implement: {ai_title} implements the message.
set -euo pipefail

{ai_function}

prompt="You are a senior Rust engineer. Read ${DECREE_MESSAGE} and
implement all requirements with proper error handling and tests.
Previous attempt logs (if any) are in ${DECREE_RUN_DIR} for context."
echo "=== AI prompt (implementation) ==="
echo "${prompt}"
ai "${prompt}"
