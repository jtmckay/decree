---
name: develop
needs:
- ai
- infra
machine:
  description: Implement a message with the AI in small, logged steps, run the project's gate, have the AI fix failures only if the gate fails, then have the AI check the acceptance criteria.
  onentry:
  - clean_tree
  initial: precheck
  states:
    precheck:
      invoke: precheck
      transitions:
        done: implement
    implement:
      invoke:
        script:
          name: implement
          attempts: 3
      transitions:
        done: gate
        stop: failed
    gate:
      invoke: gate
      transitions:
        pass: verify
        fail: fix
    fix:
      invoke:
        script:
          name: fix
          attempts: 3
      transitions:
        done: final_gate
        stop: failed
    final_gate:
      invoke: gate
      transitions:
        pass: verify
        fail: failed
    verify:
      invoke:
        script:
          name: verify
          attempts: 3
      transitions:
        pass: done
        fail: failed
        stop: failed
    done:
      final: true
      onentry:
      - commit
    failed:
      final: true
      onentry:
      - keep
scripts:
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
  fix.sh: |
    #!/usr/bin/env bash
    # develop's fix: the AI fixes what the gate reported. Like implement, it
    # writes STOP instead of guessing.
    set -euo pipefail
    . .decree/machines/ai/scripts/ai.sh

    progress="${DECREE_ROOT_RUN_DIR}/progress.md"
    plan="${DECREE_ROOT_RUN_DIR}/plan.md"
    stop="${DECREE_ROOT_RUN_DIR}/STOP"
    stopped() {
      [ -f "${stop}" ] || return 1
      cat "${stop}" >&2
      echo stop > "${DECREE_EVENT_FILE}"
    }
    stopped && exit 0

    prompt="Read ${DECREE_MESSAGE}. The project's gate
    (.decree/machines/develop/scripts/gate.sh) failed; its output is in
    ${DECREE_ROOT_RUN_DIR}/gate.log, and ${progress} notes what was done so far.
    Fix the failures, append a line to ${progress} for each fix, and run the
    gate again. If ${plan} exists, follow it. If a failure needs a decision the
    message does not make, write the question to ${stop} and stop.
    Write scratch files only under ${DECREE_RUN_DIR}, never outside the project (not /tmp)."
    echo "=== AI prompt (fix) ==="
    echo "${prompt}"
    ai "${prompt}"
    stopped || true
  gate.sh: |
    #!/usr/bin/env bash
    # develop's gate: the project's own checks, also kept in gate.log for fix. It
    # names `pass` if they all pass and `fail` if one fails; a non-zero exit, such
    # as a missing tool, means they could not run, and ends the run without fix.
    # Replace the echo with your project's checks, such as one of the commented
    # lines, and uncomment the `command -v` line for its tool. Keep the edit with
    # `decree export develop`, or let the next run's `commit` export it.
    set -euo pipefail
    log="${DECREE_ROOT_RUN_DIR}/gate.log"

    # A tool the checks need is not on PATH: they cannot run.
    missing() {
      echo "gate: $1 not found on PATH; the checks cannot run" | tee "${log}" >&2
      exit 1
    }
    # command -v cargo >/dev/null || missing cargo
    # command -v npm >/dev/null || missing npm
    # command -v go >/dev/null || missing go

    # The checks, as one && chain: the first that fails names `fail`.
    checks() {
      echo "gate: no checks configured; edit .decree/machines/develop/scripts/gate.sh, then keep it with \`decree export develop\`"
      # cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
      # npm ci && npm run lint && npm test
      # gofmt -l . | (! grep .) && go vet ./... && go test ./...
      # and, once qa has found bugs, its chaos suite:
      # (. .decree/machines/qa/scripts/qa.sh && bash -c "${QA_RUN}")
    }

    if checks 2>&1 | tee "${log}"; then
      echo pass > "${DECREE_EVENT_FILE}"
    else
      echo fail > "${DECREE_EVENT_FILE}"
    fi
  implement.sh: |
    #!/usr/bin/env bash
    # develop's implement: the AI implements the message in small steps, noting
    # each in progress.md, so a retry continues where the last attempt stopped. If
    # the message is unclear, it writes the question to STOP instead of guessing;
    # the run fails until a person answers it in plan.md, deletes STOP and runs
    # `decree daemon --once --retry`. Its notes live in the root run's folder
    # ($DECREE_ROOT_RUN_DIR, the run's own unless it is a child run), so a child machine
    # whose steps are develop's scripts shares them.
    set -euo pipefail
    . .decree/machines/ai/scripts/ai.sh

    progress="${DECREE_ROOT_RUN_DIR}/progress.md"
    plan="${DECREE_ROOT_RUN_DIR}/plan.md"
    gaps="${DECREE_ROOT_RUN_DIR}/gaps.md"
    stop="${DECREE_ROOT_RUN_DIR}/STOP"
    stopped() {
      [ -f "${stop}" ] || return 1
      cat "${stop}" >&2
      echo stop > "${DECREE_EVENT_FILE}"
    }
    stopped && exit 0

    prompt="You are a senior engineer. Read ${DECREE_MESSAGE} and
    implement all requirements with proper error handling and tests.
    Work in small steps, one requirement at a time, and keep the code building
    between steps. After each step, append a line to ${progress}: what is done
    and what is next. If ${progress} exists, an earlier attempt was cut short:
    check the code against it and continue from there. If ${plan} exists, follow it.
    If ${gaps} exists, an earlier verify found these gaps: close each one, test first.
    If the message is ambiguous or conflicts with the code, do not guess: write
    the question to ${stop} and stop.
    Logs of earlier attempts (if any) are in ${DECREE_RUN_DIR}.
    Write scratch files only under ${DECREE_RUN_DIR}, never outside the project (not /tmp)."
    echo "=== AI prompt (implementation) ==="
    echo "${prompt}"
    ai "${prompt}"
    stopped || true
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
  precheck.sh: |
    #!/usr/bin/env bash
    # Fail fast if the environment every .decree/machines/*/scripts/requirements declares is not all
    # there, such as the AI CLI or git missing from PATH. An infra message adds what is missing;
    # develop does not.
    set -euo pipefail
    .decree/machines/infra/scripts/run.sh check
  verify.sh: |
    #!/usr/bin/env bash
    # develop's verify: the AI checks the acceptance criteria and ends its reply
    # with `VERDICT: pass` or `VERDICT: fail`; the last such line is the event. A
    # reply without one fails the attempt, so decree asks again. Like implement, it
    # writes STOP instead of guessing. On `fail` it leaves gaps.md, what is missing, for
    # the next round or retry; an attempt that dies keeps the last one.
    set -euo pipefail
    . .decree/machines/ai/scripts/ai.sh

    progress="${DECREE_ROOT_RUN_DIR}/progress.md"
    plan="${DECREE_ROOT_RUN_DIR}/plan.md"
    gaps="${DECREE_ROOT_RUN_DIR}/gaps.md"
    stop="${DECREE_ROOT_RUN_DIR}/STOP"
    stopped() {
      [ -f "${stop}" ] || return 1
      cat "${stop}" >&2
      echo stop > "${DECREE_EVENT_FILE}"
    }
    stopped && exit 0

    prompt="Read ${DECREE_MESSAGE}. Verify that all requirements and
    acceptance criteria are met; ${progress} notes what was done. Run the tests,
    and report what passes and what fails. Change no code. If ${plan} exists, it
    holds decisions made since the message. If a criterion needs a decision the
    message does not make, write the question to ${stop} and stop.
    If anything fails, write ${gaps}: one line per unmet requirement or failing test,
    saying what is wrong.
    Write scratch files only under ${DECREE_RUN_DIR}, never outside the project (not /tmp).
    End your reply with one line: VERDICT: pass if everything passes, else
    VERDICT: fail."
    echo "=== AI prompt (verification) ==="
    echo "${prompt}"
    reply=$(mktemp)
    # The last gaps.md is set aside, and put back if this attempt leaves none.
    [ ! -f "${gaps}" ] || mv "${gaps}" "${gaps}.last"
    trap 'rm -f "${reply}"; [ -s "${gaps}" ] || [ ! -f "${gaps}.last" ] || mv "${gaps}.last" "${gaps}"' EXIT
    ai "${prompt}" | tee "${reply}"
    stopped && exit 0

    # The last line naming a verdict, Markdown emphasis and backticks allowed.
    verdict=$(tr -d '*`' < "${reply}" \
      | sed -nE 's/^[[:space:]]*VERDICT:[[:space:]]*(pass|fail)[[:space:]]*$/\1/p' | tail -n 1)
    if [ -z "${verdict}" ]; then
      echo "verify: no 'VERDICT: pass' or 'VERDICT: fail' line in the reply" >&2
      exit 1
    fi
    [ "${verdict}" = fail ] || rm -f "${gaps}"
    rm -f "${gaps}.last"
    echo "${verdict}" > "${DECREE_EVENT_FILE}"
---
# develop

Implements a message, such as an order file describing a change, with an AI coding agent, then runs your project's checks and has the agent check the message's acceptance criteria. Each run ends in one git commit, or in `failed` with its changes left in the work tree.

## What it does

1. `clean_tree` (on entry): the run starts from a clean git work tree, so the commit at the end holds this run's work only.
2. `precheck`: fails fast if anything that `.decree/machines/*/scripts/requirements` declares is missing, by running infra's `run.sh check` (so `develop` needs `infra`).
3. `implement`: the agent implements the message in small steps, logging each in the run's `progress.md`. If the message is ambiguous it writes `STOP` with its question instead of guessing, and the run ends in `failed`.
4. `gate`: your project's checks, `.decree/machines/develop/scripts/gate.sh`. Passing goes to `verify`; failing goes to `fix`, where the agent fixes what `gate.log` reports, then to `final_gate`, whose failure ends the run.
5. `verify`: the agent checks every acceptance criterion and ends its reply with `VERDICT: pass` or `VERDICT: fail`.
6. `done`: `commit` exports every plugin a run edited (`decree export`), then commits the run's work in one commit, titled with the message's first heading. `failed`: `keep` leaves the changes in the work tree for `decree daemon --once --retry <id>`, and holds the queue.

## What it needs

- git, and a project that is a git work tree with at least one commit.
- An AI CLI on `PATH`: `claude`, `opencode` or `copilot`. `ai.sh` uses the run's `ai_cli` data, else `AI_CLI` (in `.decree/.env`), else the first of them it finds.
- The `infra` plugin, which `decree install develop` adds with it.

## How to use it

1. Put your project's checks in `.decree/machines/develop/scripts/gate.sh` (it prints `gate: no checks configured` until you do), for instance `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`. The edit is kept in this file by `decree export develop`, or by the next run's `commit`.
2. Write an order file, `.decree/machines/develop/order/A01-<what>.md`, saying what to change and how to tell it is done.
3. Run `decree daemon --once`, or keep `decree daemon` running: the official `order` plugin runs the file at start.
