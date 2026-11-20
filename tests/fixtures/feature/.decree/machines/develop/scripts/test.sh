#!/usr/bin/env bash
# Shared invoke. Non-zero exit is the error event; with no error transition
# declared, the machine goes to failed.
set -euo pipefail
cargo test
