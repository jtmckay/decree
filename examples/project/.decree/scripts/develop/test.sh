#!/usr/bin/env bash
# develop's test. Exit 0 is done; anything else is error, which goes to
# failed, implicitly.
set -euo pipefail
cargo test
