#!/usr/bin/env bash
# Rust Develop
#
# Rust-specific development routine. Delegates implementation to AI,
# builds and tests, then hands failures to AI for fix-up.
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

# Pre-check: verify AI tool and cargo are available
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then
    command -v {AI_CMD} >/dev/null 2>&1 || { echo "{AI_CMD} not found"; exit 1; }
    command -v cargo >/dev/null 2>&1 || { echo "cargo not found"; exit 1; }
    exit 0
fi

if [ -n "$input_file" ] && [ -f "$input_file" ]; then
    WORK_FILE="$input_file"
else
    WORK_FILE="$message_file"
fi

# Step 1: Implementation
{AI_CMD} run "You are a senior Rust engineer. Read ${WORK_FILE} and implement
all requirements with proper error handling and tests."

# Step 2: Build and test
echo "=== Building (release) ==="
cargo build --release 2>&1 | tee "${message_dir}/build.log" || true
echo "=== Running tests ==="
cargo test 2>&1 | tee "${message_dir}/test-output.log" || true

# Step 3: QA
{AI_CMD} run "Read ${WORK_FILE}, build output at ${message_dir}/build.log,
test output at ${message_dir}/test-output.log. Fix any failures. Run cargo
build --release and cargo test again. Exit 0 only if everything passes."
