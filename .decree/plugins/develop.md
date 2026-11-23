---
name: develop
extends: decree_develop
needs:
- decree_infra: infra
scripts:
  conventions.md: |
    # Rust conventions (develop's implement, fix and verify prompts read this)

    - Format with `cargo fmt`; keep `cargo clippy --all-targets -- -D warnings` clean.
    - Unit tests in a `#[cfg(test)] mod tests` beside the code; behaviour a user sees
      (commands, files, exit codes) in `tests/`, driving the built binary.
    - Return errors with context; no `unwrap()` or `expect()` outside tests unless the
      invariant is proven on the line above.
    - No new dependency unless the message allows it.
    - `cargo test` must pass before you end; run the narrowest test first while working.
  gate.sh: |
    #!/usr/bin/env bash
    # develop's gate: the project's own checks, also kept in gate.log for fix. It
    # names `pass` if they all pass and `fail` if one fails; a non-zero exit, such
    # as a missing tool, means they could not run, and ends the run without fix.
    # This repository's checks: cargo fmt, clippy and the tests, one per line
    # (tests/ci_test.rs reads them to keep .github/workflows/ci.yml the same).
    set -euo pipefail
    log="${DECREE_ROOT_RUN_DIR}/gate.log"

    # A tool the checks need is not on PATH: they cannot run.
    missing() {
      echo "gate: $1 not found on PATH; the checks cannot run" | tee "${log}" >&2
      exit 1
    }
    command -v cargo >/dev/null || missing cargo

    # The checks, as one && chain: the first that fails names `fail`.
    checks() {
      cargo fmt --check &&
        cargo clippy --all-targets -- -D warnings &&
        cargo test
    }

    if checks 2>&1 | tee "${log}"; then
      echo pass > "${DECREE_EVENT_FILE}"
    else
      echo fail > "${DECREE_EVENT_FILE}"
    fi
  run-queue.sh: |
    #!/usr/bin/env bash
    # run-queue: run this repository's order files (.decree/machines/develop/order/)
    # unattended, each one on a decree built from the commit before it, since each
    # changes decree itself.
    #
    #   .decree/machines/develop/scripts/run-queue.sh
    #
    # Each pass builds the release binary and runs `ORDER_MAX=1 decree daemon --once`:
    # a daemon start, at which the official `order` plugin runs one order file and its
    # follow-ups; develop's `commit` records it in state/order.git.txt in the run's
    # commit. It ends with 0 when a pass ran no file (every file has run), and with 1
    # when a run needs a person: a file in .decree/attention/, a STOP, or a failed
    # build. Ctrl-C interrupts the run going (`decree retry <id>`).
    # The daemon runs a copy of the build, target/run-queue/decree: a run's own
    # `cargo build --release` replaces target/release/decree, and the scripts that call
    # the daemon's path (commit's `decree export`) would find it gone.
    set -euo pipefail
    cd "$(git rev-parse --show-toplevel)"
    attention=.decree/attention
    record=.decree/machines/develop/state/order.git.txt
    decree=./target/run-queue/decree

    while :; do
      echo "run-queue: building decree at $(git log -1 --format='%h %s')"
      cargo build --release --quiet || { echo "run-queue: the build failed; fix it and run me again" >&2; exit 1; }
      mkdir -p "$(dirname "${decree}")"
      cp ./target/release/decree "${decree}.new" && mv -f "${decree}.new" "${decree}"
      before=$(cat "${record}" 2>/dev/null || true)
      ORDER_MAX=1 "${decree}" daemon --once || true
      if find "${attention}" -maxdepth 1 -name '*.md' 2>/dev/null | grep -q .; then
        echo "run-queue: a run needs you (${attention}/):" >&2
        "${decree}" status >&2 || true
        exit 1
      fi
      if [ "$(cat "${record}" 2>/dev/null || true)" = "${before}" ]; then
        echo "run-queue: nothing more to run"
        exit 0
      fi
    done
---
# develop

This repository's develop: [`decree_develop`](../../plugins/decree_develop.md), extended with this repository's gate (`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, which `tests/ci_test.rs` keeps equal to `.github/workflows/ci.yml`), its Rust conventions, and `run-queue.sh`, which runs the order files in `.decree/machines/develop/order/` unattended, each on a decree built from the commit before it. Its `needs` map sends `precheck` to this repository's `infra`.
