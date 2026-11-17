#!/usr/bin/env bash
# qa's attack: the AI writes chaos tests in QA_DIR, each naming its wrench and invariant.
set -euo pipefail
. .decree/scripts/qa/ai.sh
. .decree/scripts/qa/qa.sh
qa_ready
qa_guard

prompt="You are QA. Your job is to make this project misbehave. You are done when you have
written tests that fail, or have tried every wrench in .decree/scripts/qa/wrenches.md.
Read ${DECREE_RUN_DIR}/surface.md, .decree/scripts/qa/invariants.md and the tests already in
${QA_DIR}/ (do not repeat a wrench and invariant pair tested there). Write new tests in
${QA_DIR}/, in the project's language and test framework, run by: ${QA_RUN}
Each test names its wrench and the invariant it checks. Seed every random choice with a
constant. A test asserts the invariant, never what the code happens to do now.
Change only files in ${QA_DIR}/: you find bugs, you never fix them. Do not run the tests.
Write scratch files only under ${DECREE_RUN_DIR}, never outside the project (not /tmp)."
echo "=== AI prompt (attack) ==="
echo "${prompt}"
ai "${prompt}"
qa_check
