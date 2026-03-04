#!/usr/bin/env bash
# Develop
#
# Default routine that delegates work to an AI assistant.
# Reads the input file or task message, prompts the AI to implement all
# requirements, then verifies acceptance criteria are met.
set -euo pipefail

# --- Parameters ---
# message_file  - Path to message.md in the run directory
# message_id    - Full message ID (<chain>-<seq>)
# message_dir   - Run directory path
# chain         - Chain ID
# seq           - Sequence number
# input_file    - Optional. Path to input file (e.g., migration file)
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
input_file="${input_file:-}"

# Pre-check: verify AI tool is available
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then
    command -v {AI_CMD} >/dev/null 2>&1 || { echo "{AI_CMD} not found"; exit 1; }
    exit 0
fi

# Determine the work description
if [ -n "$input_file" ] && [ -f "$input_file" ]; then
    WORK_FILE="$input_file"
else
    WORK_FILE="$message_file"
fi

# Implementation
{AI_CMD} run "Read the work description at ${WORK_FILE} and implement all
requirements. Follow best practices: clean code, proper error handling,
and tests where appropriate."

# Verification
{AI_CMD} run "Read the work description at ${WORK_FILE}. Verify that all
requirements and acceptance criteria are met. Run any tests. Report what
passes and what fails. Exit 0 if everything passes, exit 1 if anything fails."
