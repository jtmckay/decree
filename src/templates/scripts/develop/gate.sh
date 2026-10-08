#!/usr/bin/env bash
# develop's gate: the project's own checks, also kept in gate.log for fix. A
# non-zero exit sends the run to fix. Replace the echo with your project's
# checks, such as one of the commented lines.
set -euo pipefail
{
  echo "gate: no checks configured; edit .decree/scripts/develop/gate.sh"
  # cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
  # npm ci && npm run lint && npm test
  # gofmt -l . | (! grep .) && go vet ./... && go test ./...
} 2>&1 | tee "${DECREE_RUN_DIR}/gate.log"
