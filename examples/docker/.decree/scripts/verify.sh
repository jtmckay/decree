#!/usr/bin/env bash
# Have opencode check the acceptance criteria and fix anything that fails.
set -euo pipefail
opencode run "Read ${DECREE_MESSAGE} and verify all acceptance criteria are met.
If anything fails, fix it."
