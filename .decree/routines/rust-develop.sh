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
    command -v claude >/dev/null 2>&1 || { echo "claude not found" >&2; exit 1; }
    command -v cargo >/dev/null 2>&1 || { echo "cargo not found" >&2; exit 1; }
    command -v uuidgen >/dev/null 2>&1 || { echo "uuidgen not found" >&2; exit 1; }
    exit 0
fi

stop_file="${message_dir}/STOP"
stop_if_requested() {
    if [ -f "${stop_file}" ]; then
        echo "=== Agent requested a stop (${stop_file}) ===" >&2
        cat "${stop_file}" >&2
        exit 1
    fi
}
# A STOP from an earlier attempt stays in force until a human removes it.
stop_if_requested

# Every claude session gets a known id, recorded in sessions.txt, so its full
# transcript (reasoning and tool calls, written as it goes, kept even if the
# session dies) can be found from the run directory, and later steps resume it.
transcripts="${HOME}/.claude/projects/$(pwd | sed 's/[^A-Za-z0-9]/-/g')"
new_session() {
    local id
    id="$(uuidgen)"
    echo "$1 ${id} ${transcripts}/${id}.jsonl" >> "${message_dir}/sessions.txt"
    echo "=== $1 session ${id} ===" >&2
    echo "${id}"
}

gate() {
    cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
}

progress="${message_dir}/progress.md"
work_rules="Work in small steps, one requirement or acceptance criterion at a time,
and keep the tree compiling between steps. After each step, append a line to
${progress}: what is done (files, test names) and what is next. If ${progress}
already exists, an earlier attempt was cut short: read it, check the current
code against it, and continue from there instead of starting over."

# Step 1: Implementation
impl_session="$(new_session implement)"
claude --permission-mode auto --session-id "${impl_session}" -p "You are a senior Rust engineer. Read ${message_file} and
implement all requirements with proper error handling and tests.
${work_rules}
Previous attempt logs (if any) are in ${message_dir} for context.
The run directory is ${message_dir}."
stop_if_requested

# Step 2: Gate. claude -p exits 0 whatever the agent concludes, so the
# routine decides success itself. Passing here skips the QA session.
echo "=== Gate: fmt, clippy, test ==="
if gate > "${message_dir}/gate.log" 2>&1; then
    tail -5 "${message_dir}/gate.log"
    exit 0
fi
tail -40 "${message_dir}/gate.log"

# Step 3: QA, in the implementing session, so it keeps what it already read.
echo "=== QA (resuming ${impl_session}) ===" >&2
echo "qa ${impl_session} (resumed)" >> "${message_dir}/sessions.txt"
claude --permission-mode auto --resume "${impl_session}" -p "The gate failed; its output is in ${message_dir}/gate.log.
Fix the failures and run cargo fmt --check, cargo clippy --all-targets -- -D warnings
and cargo test again. ${work_rules}"
stop_if_requested

# Step 4: Final gate; a failure here retries or stops the queue.
echo "=== Gate: fmt, clippy, test ==="
gate
