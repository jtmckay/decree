#!/usr/bin/env bash
# Rust Develop
#
# Rust-specific development routine. Delegates implementation to an AI
# assistant, builds and tests the result, then hands build/test output
# to a QA engineer AI to diagnose and fix any failures. Use this routine
# for Rust projects where you want cargo build --release and cargo test
# as the verification gate with automated fix-up.
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

# Step 1: Implementation
{AI_CMD} "You are a senior Rust engineer. Read the work description at ${WORK_FILE}.
Implement all requirements. Follow Rust best practices: proper error
handling with Result/Option, no unwrap in library code, idiomatic types,
and tests where appropriate. Work methodically through each requirement." \
  {ALLOWED_TOOLS}

# Step 2: Build and test
echo "=== Building (release) ==="
cargo build --release 2>&1 | tee "${message_dir}/build.log" || true

echo "=== Running tests ==="
cargo test 2>&1 | tee "${message_dir}/test-output.log" || true

# Step 3: QA — diagnose and fix failures
{AI_CMD} "You are an expert Quality Assurance Engineer for a Rust project.

Read the work description at ${WORK_FILE}.
Read the build output at ${message_dir}/build.log.
Read the test output at ${message_dir}/test-output.log.

If the build or tests passed cleanly, verify the implementation meets all
requirements and acceptance criteria from the work description — exit 0.

If there are build errors or test failures:
1. Diagnose each failure — read the relevant source and test code.
2. Apply the minimal fix. Prefer fixing source code over weakening tests.
3. Do NOT add new features or refactor unrelated code.
4. Run cargo build --release and cargo test again to confirm everything passes.
Exit 0 only if the build succeeds and all tests pass." \
  {ALLOWED_TOOLS}
