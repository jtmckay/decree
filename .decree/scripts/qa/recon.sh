#!/usr/bin/env bash
# qa's recon: Claude lists the attack surface, from the docs and the code, in surface.md.
set -euo pipefail
. .decree/scripts/qa/ai.sh
. .decree/scripts/qa/qa.sh
qa_ready
qa_guard

prompt="You are QA. Read the project's docs and code, .decree/scripts/qa/invariants.md and
.decree/scripts/qa/wrenches.md. Write ${DECREE_RUN_DIR}/surface.md: a short list of the
attack surface (each input, each piece of saved state, each call to the outside world,
each limit), and for each, the invariants it could break. Change no other file.
Write scratch files only under ${DECREE_RUN_DIR}, never outside the project (not /tmp)."
echo "=== AI prompt (recon) ==="
echo "${prompt}"
ai "${prompt}"
qa_check
