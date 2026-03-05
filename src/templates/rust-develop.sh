#!/usr/bin/env bash
# Rust Develop
#
# Rust-specific development routine. Delegates implementation to AI,
# builds and tests, then hands failures to AI for fix-up.
set -euo pipefail

# --- Standard Environment Variables ---
# message_file  - Path to message.md in the run directory
# message_id    - Full message ID (e.g., D0001-1432-01-add-auth-0)
# message_dir   - Run directory path (contains logs from prior attempts)
# chain         - Chain ID (D<NNNN>-HHmm-<name>)
# seq           - Sequence number in chain
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"

# Pre-check: verify AI tool and cargo are available
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then
    command -v {ai_name} >/dev/null 2>&1 || { echo "{ai_name} not found" >&2; exit 1; }
    command -v cargo >/dev/null 2>&1 || { echo "cargo not found" >&2; exit 1; }
    exit 0
fi

# Step 1: Implementation
{ai_invoke} "You are a senior Rust engineer. Read ${message_file} and
implement all requirements with proper error handling and tests.
Previous attempt logs (if any) are in ${message_dir} for context."

# Step 2: Build and test
echo "=== Building (release) ==="
cargo build --release 2>&1 | tee "${message_dir}/build.log" || true
echo "=== Running tests ==="
cargo test 2>&1 | tee "${message_dir}/test-output.log" || true

# Step 3: QA
{ai_invoke} "Read ${message_file}, build output at ${message_dir}/build.log,
test output at ${message_dir}/test-output.log. Fix any failures. Run cargo
build --release and cargo test again. Exit 0 only if everything passes."
