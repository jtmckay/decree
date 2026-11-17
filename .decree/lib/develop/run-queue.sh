#!/usr/bin/env bash
# run-queue: run this repository's pending migrations unattended, each one on a
# decree built from the commit before it.
#
#   .decree/lib/develop/run-queue.sh
#
# develop's `restart` (done's onentry, after commit) runs `decree stop`, so
# `process` exits after each migration. This loop then builds the release
# binary, deletes the stop file and runs `process` again with the new build.
# It ends with 0 when a pass ran no migration (the queue is empty), and with 1
# when a run needs a person: a failed run's attention file, a STOP, or a
# failed build. Ctrl-C interrupts the run going (`decree process --retry`).
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
attention=.decree/attention
stop="${attention}/manual-stop.md"
decree=./target/release/decree

while :; do
  echo "run-queue: building decree at $(git log -1 --format='%h %s')"
  cargo build --release --quiet || { echo "run-queue: the build failed; fix it and run me again" >&2; exit 1; }
  rm -f "${stop}"
  "${decree}" process || true
  if find "${attention}" -maxdepth 1 -name '*.md' ! -name manual-stop.md 2>/dev/null | grep -q .; then
    echo "run-queue: a run needs you (${attention}/):" >&2
    "${decree}" status >&2 || true
    exit 1
  fi
  if [ ! -f "${stop}" ]; then
    echo "run-queue: nothing more to run"
    exit 0
  fi
done
