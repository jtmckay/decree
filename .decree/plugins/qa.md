---
name: qa
needs:
- ai
- develop
machine:
  description: Attack the project to make it misbehave, and hand each real bug to develop as a failing test; the AI writes tests only and never fixes code.
  onentry:
  - clean_tree
  initial: recon
  states:
    recon:
      invoke: recon
      transitions:
        done: attack
    attack:
      invoke: attack
      transitions:
        done: run
    run:
      invoke: run
      transitions:
        done: triage
    triage:
      invoke: triage
      transitions:
        done: report
        stop: failed
    report:
      invoke: report
      transitions:
        done: done
      emits:
      - develop
    done:
      final: true
      onentry:
      - commit
    failed:
      final: true
      onentry:
      - keep
scripts:
  attack.sh: |
    #!/usr/bin/env bash
    # qa's attack: the AI writes chaos tests in QA_DIR, each naming its wrench and invariant.
    set -euo pipefail
    . .decree/machines/ai/scripts/ai.sh
    . .decree/machines/qa/scripts/qa.sh
    qa_ready
    qa_guard

    prompt="You are QA. Your job is to make this project misbehave. You are done when you have
    written tests that fail, or have tried every wrench in .decree/machines/qa/scripts/wrenches.md.
    Read ${DECREE_RUN_DIR}/surface.md, .decree/machines/qa/scripts/invariants.md and the tests already in
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
  clean_tree.sh: |
    #!/usr/bin/env bash
    # Root onentry script: the run starts from a clean git work tree, so the commit at `done`
    # holds this run's work only, and a failed run's changes never mix into the next one.
    # It runs again when `decree daemon --once --retry` continues the run; by then the tree holds
    # the run's own work, so it checks only the first time. Only the project folder counts
    # (`-- .`): a decree project can be a subfolder of a larger repository.
    # When it finds changes, it names each run that passed here and is now `interrupted` or
    # `failed` (an interrupted run never reaches `keep`), with the commands that continue it or
    # set its changes aside, and writes those lines to the run's `leftovers`, which decree
    # quotes in this run's file in .decree/attention/. It reads `decree status --format json`
    # with `jq`; without `jq` on PATH it names no run.
    set -euo pipefail
    mark="${DECREE_RUN_DIR}/clean_tree"
    leftovers="${DECREE_RUN_DIR}/leftovers"
    [ -f "${mark}" ] && exit 0
    command -v git >/dev/null || { echo "clean_tree: git not found on PATH" >&2; exit 1; }
    git rev-parse --is-inside-work-tree >/dev/null 2>&1 || {
      echo "clean_tree: not a git work tree; run \`git init\` and commit first" >&2
      exit 1
    }

    # The runs that passed `clean_tree` and stopped before `done`, one line each:
    # `<id> <status>`.
    stopped_runs() {
      command -v jq >/dev/null || return 0
      local json
      json=$("${DECREE}" status --all --format json 2>/dev/null) || return 0
      jq -r '(.runs.interrupted // [] | map([.id, "interrupted"]))
        + (.runs.finished.failed // [] | map([.id, "failed"]))
        | .[] | @tsv' <<<"${json}" 2>/dev/null || return 0
    }

    # What to tell the person about each run that may have left the changes.
    named_runs() {
      local id status dir title
      while IFS=$'\t' read -r id status; do
        [ -n "${id}" ] && [ "${id}" != "${DECREE_MESSAGE_ID:-}" ] || continue
        dir=".decree/runs/${id}"
        [ -f "${dir}/clean_tree" ] || continue
        title=$(sed -n 's/^# //p' "${dir}/message.md" 2>/dev/null | head -n 1)
        echo "run ${id} (${title:-no heading}) started from a clean tree and is ${status}; it may have left them:"
        echo "  to continue it on them: \`decree daemon --once --retry ${id}\`"
        echo "  to set them aside: \`git stash push --include-untracked -m \"decree ${id}\" -- .\`"
      done < <(stopped_runs)
    }

    rm -f "${leftovers}"
    changes=$(git status --porcelain -- .)
    if [ -n "${changes}" ]; then
      echo "clean_tree: uncommitted changes; commit or stash them, then run the message again:" >&2
      echo "${changes}" | head -n 20 >&2
      n=$(echo "${changes}" | wc -l)
      [ "${n}" -le 20 ] || echo "… and $((n - 20)) more" >&2
      named=$(named_runs)
      if [ -n "${named}" ]; then
        echo "${named}" >&2
        echo "${named}" > "${leftovers}"
      fi
      echo "if a failed run left them, its file in .decree/attention/ names it; continue it with \`decree daemon --once --retry <id>\`" >&2
      exit 1
    fi
    git rev-parse HEAD > "${mark}"
    echo "clean at $(git rev-parse --short HEAD)"
  commit.sh: |
    #!/usr/bin/env bash
    # onentry script of `done`: commit the run's work, titled with the message's first `# `
    # heading. For a run of an order file (`order: <machine>/<file>` in its message.md), it
    # records the file in its machine's state/order.git.txt first, so the one commit holds the
    # work and the record, and the title ends with the file. Nothing changed is
    # nothing to commit. If the commit fails, the run ends in `failed`, not `done`. Only the
    # project folder is committed (`-- .`), even if something else in a larger repository is
    # staged.
    # First it exports every plugin whose installed copy differs from its plugin file
    # (`decree export` with no name), so what the run changed in machines/ or scripts/
    # (an infra run's new requirement, a develop run's gate) reaches .decree/plugins/ and is in
    # this one commit; git ignores the installed machines/. A failed run exports nothing: `keep`
    # leaves its edits in the installed copy, where a retry builds on them.
    set -euo pipefail
    "${DECREE}" export
    order=$(sed -n '2,/^---$/s/^order: //p' "${DECREE_MESSAGE}" | head -n 1)
    record=.decree/machines/order/scripts/record.sh
    if [ -n "${order}" ] && [ -x "${record}" ]; then
      "${record}" "${order%%/*}" "${order#*/}"
    fi
    git add -A -- .
    if git diff --cached --quiet -- .; then
      echo "nothing to commit"
      exit 0
    fi
    title=$(sed -n 's/^# //p' "${DECREE_MESSAGE}" | head -n 1)
    title=${title:-${DECREE_MACHINE}: ${DECREE_MESSAGE_ID}}
    if [ -n "${order}" ]; then
      title="${title} (${order})"
    fi
    git commit -q -m "${title}" \
      -m "decree run ${DECREE_MESSAGE_ID} (${DECREE_MACHINE})" -- .
    git log -1 --format='committed %h %s'
  invariants.md: |
    # Invariants

    What must always hold, whatever a user or the world does. qa's tests assert these, never
    what the code happens to do now. Number them, so a test and a bug report can name one.
    Until this lists any, qa takes them from the project's docs and code.

    <!-- For example:
    - I1: Saved data is never lost: after a crash or restart, the user is where they were.
    - I2: Every error the user can cause shows a message and leaves the app usable.
    - I3: Nothing the user does twice in a row has a different effect than doing it once.
    -->
  keep.sh: |
    #!/usr/bin/env bash
    # onentry script of `failed`: leave the run's changes in the work tree, so a retry builds on
    # the last attempt (the agent sees them in `git status` and decides what to keep), and write
    # the run's `hold`, so its file in .decree/attention/ holds the queue and no other run starts
    # on them. `decree daemon --once --retry` of this run removes both; to set the changes aside
    # instead, stash them and delete the run's attention file.
    # Only the project folder counts (`-- .`): a decree project can be a subfolder of a larger
    # repository. A run that failed because a person's own changes were there from the start
    # (`clean_tree` did not pass) leaves them alone and holds nothing.
    # Edits to an installed plugin (machines/, scripts/, which git ignores) count too: a failed
    # run exports nothing, so they stay in the installed copy, and the hold keeps the next run's
    # `commit` from exporting them (`decree export --dry-run` names them).
    set -euo pipefail
    [ -f "${DECREE_RUN_DIR}/clean_tree" ] || exit 0
    changes=$(git status --porcelain -- .)
    edited=$("${DECREE}" export --dry-run | sed -n 's/^would write //p')
    if [ -z "${changes}${edited}" ]; then
      echo "no changes to keep"
      exit 0
    fi
    title=$(sed -n 's/^# //p' "${DECREE_MESSAGE}" | head -n 1)
    attention=".decree/attention/${DECREE_MESSAGE_ID}.md"
    {
      echo "run ${DECREE_MESSAGE_ID} (${title:-${DECREE_MACHINE}}) failed and kept its changes in the work tree:"
      [ -z "${changes}" ] || echo "${changes}"
      [ -z "${edited}" ] || echo "${edited}" | sed 's/^/installed copy edited, not exported: /'
      echo "to continue the run on them: \`decree daemon --once --retry ${DECREE_MESSAGE_ID}\`"
      echo "to set them aside instead: \`git stash push --include-untracked -m \"decree ${DECREE_MESSAGE_ID}\" -- .\` (and \`decree install <name>\` for an installed copy), then delete ${attention}"
    } > "${DECREE_RUN_DIR}/hold"
    cat "${DECREE_RUN_DIR}/hold"
    echo "no other run starts until then (${attention})"
  qa.sh: |
    # qa's settings and its guard, which qa's scripts source.
    #
    # Where the chaos tests live, apart from the project's other tests.
    QA_DIR=tests/chaos
    # The command that runs the chaos suite and exits non-zero if a test fails. qa does not
    # start until it is set. Once qa has found bugs, run it in develop's gate too, so a
    # fixed bug whose test is still marked expected to fail is caught. Keep an edit to this
    # file with `decree export qa`, or let the next run's `commit` export it.
    QA_RUN="cargo test --test chaos"
    # QA_RUN="python3 -m unittest discover -s ${QA_DIR} -t . -v"
    # QA_RUN="npx playwright test ${QA_DIR}"
    # QA_RUN="cargo test --test chaos"
    # QA_RUN="go test ./${QA_DIR}/..."

    # Fail unless QA_RUN is set and this is a git work tree (the guard needs git).
    qa_ready() {
      if [ -z "${QA_RUN}" ]; then
        echo "qa: no chaos suite command; set QA_RUN in .decree/machines/qa/scripts/qa.sh, then keep it with \`decree export qa\`" >&2
        return 1
      fi
      git rev-parse --is-inside-work-tree >/dev/null 2>&1 || {
        echo "qa: not a git work tree; qa's guard needs git" >&2
        return 1
      }
    }

    # QA never fixes code. qa_guard hashes every file outside QA_DIR and .decree/ before the
    # AI runs; qa_check fails the step, naming each file the AI changed. It restores nothing,
    # so it never overwrites work of yours.
    qa_hashes() {
      git ls-files -z -c -o --exclude-standard -- . ":(exclude)${QA_DIR}" ":(exclude).decree" |
        xargs -0 -r sha1sum 2>/dev/null | sort
    }
    qa_guard() { qa_hashes > "${DECREE_RUN_DIR}/guard.txt"; }
    qa_check() {
      local changed
      changed=$(qa_hashes | { diff "${DECREE_RUN_DIR}/guard.txt" - || true; } |
        sed -n 's/^[<>] [0-9a-f]*  //p' | sort -u)
      if [ -n "${changed}" ]; then
        echo "qa: the AI changed files outside ${QA_DIR}; QA writes tests only:" >&2
        echo "${changed}" >&2
        return 1
      fi
    }
  recon.sh: |
    #!/usr/bin/env bash
    # qa's recon: the AI lists the attack surface, from the docs and the code, in surface.md.
    set -euo pipefail
    . .decree/machines/ai/scripts/ai.sh
    . .decree/machines/qa/scripts/qa.sh
    qa_ready
    qa_guard

    prompt="You are QA. Read the project's docs and code, .decree/machines/qa/scripts/invariants.md and
    .decree/machines/qa/scripts/wrenches.md. Write ${DECREE_RUN_DIR}/surface.md: a short list of the
    attack surface (each input, each piece of saved state, each call to the outside world,
    each limit), and for each, the invariants it could break. Change no other file.
    Write scratch files only under ${DECREE_RUN_DIR}, never outside the project (not /tmp)."
    echo "=== AI prompt (recon) ==="
    echo "${prompt}"
    ai "${prompt}"
    qa_check
  report.sh: |
    #!/usr/bin/env bash
    # qa's report: one develop message per bug report in bugs/. Safe to re-run: a report
    # already queued is listed in emitted.txt.
    set -euo pipefail
    emitted="${DECREE_RUN_DIR}/emitted.txt"
    touch "${emitted}"
    shopt -s nullglob
    for bug in "${DECREE_RUN_DIR}"/bugs/*.md; do
      grep -qxF "${bug}" "${emitted}" && continue
      id=$("${DECREE}" emit --machine develop < "${bug}")
      echo "${bug}" >> "${emitted}"
      echo "queued ${id}: $(head -n 1 "${bug}")"
    done
  run.sh: |
    #!/usr/bin/env bash
    # qa's run: the chaos suite, into chaos.log. A failing test is a finding, so this exits 0
    # either way; only a suite that could not start at all (command not found) is an error.
    set -uo pipefail
    . .decree/machines/qa/scripts/qa.sh
    bash -c "${QA_RUN}" 2>&1 | tee "${DECREE_RUN_DIR}/chaos.log"
    status=${PIPESTATUS[0]}
    if [ "${status}" -ge 126 ]; then
      echo "qa: the chaos suite could not run (exit ${status}): ${QA_RUN}" >&2
      exit 1
    fi
    exit 0
  triage.sh: |
    #!/usr/bin/env bash
    # qa's triage: the AI sorts each failure into a real bug, a wrong test or acceptable
    # behaviour. A bug's tests are marked expected to fail and get one report per root cause in
    # bugs/; wrong tests are rewritten; acceptable ones are removed. The chaos suite must then
    # pass, or the step fails. A failure the docs do not settle is a question in STOP; the run
    # fails until a person answers it in plan.md, deletes STOP and runs `decree daemon --once --retry`.
    set -euo pipefail
    . .decree/machines/ai/scripts/ai.sh
    . .decree/machines/qa/scripts/qa.sh

    bugs="${DECREE_RUN_DIR}/bugs"
    plan="${DECREE_RUN_DIR}/plan.md"
    stop="${DECREE_RUN_DIR}/STOP"
    stopped() {
      [ -f "${stop}" ] || return 1
      cat "${stop}" >&2
      echo stop > "${DECREE_EVENT_FILE}"
    }
    stopped && exit 0
    mkdir -p "${bugs}"
    qa_guard

    prompt="You are QA triage. Read ${DECREE_RUN_DIR}/chaos.log, the project's docs,
    .decree/machines/qa/scripts/invariants.md, and ${plan} if it exists. For each failing test in
    ${QA_DIR}/, decide:
    - Real bug: the project breaks an invariant or its docs. Mark the test expected to fail,
      with the framework's strict marker, so the suite passes now and fails once the bug is
      fixed (Python @unittest.expectedFailure or pytest xfail(strict=True), Playwright
      test.fail(), Jest test.failing, Rust #[should_panic]).
    - Wrong test: it asserts something the docs do not require, or no correct code could pass
      it. Rewrite it to check the invariant, or delete it.
    - Acceptable behaviour: delete the test.
    - Unclear: the docs and invariants do not say. Write the question to ${stop} and stop.
    Then group the real bugs by root cause: tests that fail for the same reason are one bug.
    For each root cause write ${bugs}/<short-slug>.md for the developer: a title line naming
    the broken behaviour; '## Overview': the invariant broken, every test it covers (file and
    name), the wrench, and what happened (the error from chaos.log); '## Outcome': the expected
    behaviour, as the invariant and the docs state it; '## Constraints': the tests' assertions
    do not change; '## Acceptance Criteria': Given those tests, When the chaos suite runs
    (${QA_RUN}), Then they pass with their expected-failure marker removed and their
    assertions unchanged. State what is broken and what is expected, never a fix: name no
    code, file or function to change and no way to change it.
    Write one line per test to ${DECREE_RUN_DIR}/triage.md: test, verdict, root cause, why.
    Change only files in ${QA_DIR}/ and ${DECREE_RUN_DIR}/. Do not run the tests.
    Write scratch files only under ${DECREE_RUN_DIR}, never outside the project (not /tmp)."
    echo "=== AI prompt (triage) ==="
    echo "${prompt}"
    ai "${prompt}"
    stopped && exit 0
    qa_check
    if ! bash -c "${QA_RUN}" > "${DECREE_RUN_DIR}/chaos-after.log" 2>&1; then
      tail -n 20 "${DECREE_RUN_DIR}/chaos-after.log" >&2
      echo "qa: the chaos suite still fails; every failure must be marked, rewritten or removed" >&2
      exit 1
    fi
  wrenches.md: |
    # Wrenches

    Ways to make the project misbehave. qa's tests each name one. Add the project's own.

    - W1 Input boundaries: empty, one, huge, negative, zero, non-ASCII, wrong type, malformed.
    - W2 Repetition: the same action twice, fast; double submit; repeat until a limit.
    - W3 Ordering: steps out of order, back and forward, an action before setup is done.
    - W4 Interruption: a crash or kill mid-write, then a restart.
    - W5 Resources: disk or quota full, no network, a slow or failing service, a timeout.
    - W6 Corrupt state: saved data truncated, garbled, from an older version, or missing.
    - W7 Concurrency: two users or processes acting on the same thing at once.
    - W8 Time: clock changes, time zones and daylight saving, dates far in the past or future.
---
# qa

Attacks your project to make it misbehave, and hands each real bug to `develop` as a failing test. The AI agent writes tests only and never fixes code.

## What it does

1. `recon`: lists the attack surface, from the docs and the code, in the run's `surface.md`. It fails first if `QA_RUN` in `.decree/machines/qa/scripts/qa.sh` is empty.
2. `attack`: writes chaos tests in `QA_DIR`, each naming the wrench (`wrenches.md`) and the invariant (`invariants.md`) it tests.
3. `run`: runs the chaos suite, `QA_RUN`, into `chaos.log`; a failing test is a finding, not an error.
4. `triage`: one bug report per root cause, its tests marked as expected to fail. It writes `STOP` instead of guessing.
5. `report`: one `develop` message per bug report.
6. `done` commits (exporting the edited plugin first); `failed` keeps the changes for a retry.

## What it needs

- The `develop` plugin (and so `infra`), which `decree install qa` adds with it.
- git, and an AI CLI on `PATH` (`claude`, `opencode` or `copilot`; see `AI_CLI`).

## How to use it

1. Set `QA_DIR` and `QA_RUN` in `.decree/machines/qa/scripts/qa.sh`: where the chaos tests go, and the command that runs them (`cargo test --test chaos`, say).
2. Write your project's invariants in `.decree/machines/qa/scripts/invariants.md`, and adjust `wrenches.md`.
3. `echo "Attack the upload API" | decree emit --machine qa && decree daemon --once`.

Edits are kept in this file by `decree export qa`, or by the next run's `commit`.
