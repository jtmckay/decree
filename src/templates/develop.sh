#!/usr/bin/env bash
# Develop
#
# Default routine that delegates work to an AI assistant.
# Reads the spec file or task message, prompts the AI to implement all
# requirements, then verifies acceptance criteria are met.
set -euo pipefail

# Parameters (decree injects these as env vars)
spec_file="${spec_file:-}"
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"

# Determine the work description
if [ -n "$spec_file" ] && [ -f "$spec_file" ]; then
    WORK_FILE="$spec_file"
else
    WORK_FILE="$message_file"
fi

# Implementation
{AI_CMD} "You are a senior software engineer. Read the work description at ${WORK_FILE} and implement all requirements. Follow best practices: clean code, proper error handling, and tests where appropriate. Work methodically through each requirement." \
  {ALLOWED_TOOLS}

# Verification
{AI_CMD} "Read the work description at ${WORK_FILE}. Verify that all requirements and acceptance criteria are met. Run any tests defined in the project. Clean up unused code: remove dead imports, unused functions, unreachable branches, and orphaned helpers introduced during implementation. Report what passes and what fails. Exit 0 if everything passes, exit 1 if anything fails." \
  {ALLOWED_TOOLS}
