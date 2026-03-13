#!/usr/bin/env bash
# Develop
#
# Default development routine. Sends the spec to an AI tool for
# implementation, then sends it again for verification against
# acceptance criteria.
set -euo pipefail

# --- Standard Environment Variables ---
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"

# Pre-check
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then
    command -v opencode >/dev/null 2>&1 || { echo "opencode not found" >&2; exit 1; }
    exit 0
fi

# Implement
opencode run "Read ${message_file} and implement the requirements.
Previous attempt logs (if any) are in ${message_dir} for context."

# Verify
opencode run "Read ${message_file} and verify all acceptance criteria are met.
If anything fails, fix it."
