#!/usr/bin/env bash
# Raw
#
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

claude -p "${WORK_FILE}"
