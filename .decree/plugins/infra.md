---
name: infra
needs:
- ai
machine:
  description: Add to the environment the project needs, the first time and every time after; the AI declares each resource in machines/infra/scripts/requirements, then apply installs what is missing and checks every line.
  concurrency_group: worktree
  onentry:
  - clean_tree
  initial: declare
  states:
    declare:
      invoke: declare
      transitions:
        done: apply
        stop: failed
    apply:
      invoke: apply
      transitions:
        done: done
    done:
      final: true
      onentry:
      - commit
    failed:
      final: true
      onentry:
      - keep
scripts:
  apply.sh: |
    #!/usr/bin/env bash
    # infra's apply: install what every machine's requirements declare and is missing.
    exec .decree/machines/infra/scripts/run.sh apply
  clean_tree.sh: |
    #!/usr/bin/env bash
    # Root onentry script: the run starts from a clean git work tree, so the commit at `done`
    # holds this run's work only, and a failed run's changes never mix into the next one.
    # It runs again when `decree retry` continues the run; by then the tree holds
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
        echo "  to continue it on them: \`decree retry ${id}\`"
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
      echo "if a failed run left them, its file in .decree/attention/ names it; continue it with \`decree retry <id>\`" >&2
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
  declare.sh: |
    #!/usr/bin/env bash
    # infra's declare: the AI changes machines/infra/scripts/requirements to meet the message, and
    # installs nothing; apply does. If the change needs a choice the message does not make,
    # it writes the question to STOP instead of guessing; the run fails until a person
    # answers it in plan.md, deletes STOP and runs `decree retry`.
    set -euo pipefail
    . .decree/machines/ai/scripts/ai.sh

    infra=.decree/machines/infra/scripts
    plan="${DECREE_RUN_DIR}/plan.md"
    stop="${DECREE_RUN_DIR}/STOP"
    stopped() {
      [ -f "${stop}" ] || return 1
      cat "${stop}" >&2
      echo stop > "${DECREE_EVENT_FILE}"
    }
    stopped && exit 0

    prompt="You maintain this project's environment. Read ${DECREE_MESSAGE}.
    The environment is declared: ${infra}/requirements lists each resource the project
    needs (kind, name, version, source), in order, and ${infra}/kinds/<kind>.sh says how
    to check and install that kind. Read both, then change the lines of requirements to
    meet the message. Put a line after the lines it relies on, and pin every version the
    kind takes. Only if no kind fits, add one in ${infra}/kinds/, a check and an install
    function like the others: it installs into the project, never with sudo, and a step
    only a person can do (root, an account) prints the command and returns 1. A kind that
    downloads a file checks it against the sha256 in its line's source (<url>#sha256=<hex>)
    and fails on a mismatch; take the hash from what the project publishes (a SHA256SUMS
    file, the release page), never by downloading the file and hashing it yourself.
    Change nothing else. Do not run anything that installs; the next step does. You may
    look at this host read-only to find what a line relies on, and declare that too.
    If ${plan} exists, follow it. If the message needs a choice it does not make, do not
    guess: write the question to ${stop} and stop.
    Write scratch files only under ${DECREE_RUN_DIR}, never outside the project (not /tmp)."
    echo "=== AI prompt (declare) ==="
    echo "${prompt}"
    ai "${prompt}"
    stopped || true
  keep.sh: |
    #!/usr/bin/env bash
    # onentry script of `failed`: leave the run's changes in the work tree, so a retry builds on
    # the last attempt (the agent sees them in `git status` and decides what to keep), and write
    # the run's `hold`, so its file in .decree/attention/ holds the work tree's group and no
    # other run in it starts on them. `decree retry` of this run removes both; to set the changes aside
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
      echo "to continue the run on them: \`decree retry ${DECREE_MESSAGE_ID}\`"
      echo "to set them aside instead: \`git stash push --include-untracked -m \"decree ${DECREE_MESSAGE_ID}\" -- .\` (and \`decree install <name>\` for an installed copy), then delete ${attention}"
    } > "${DECREE_RUN_DIR}/hold"
    cat "${DECREE_RUN_DIR}/hold"
    echo "no other run in the work tree's group starts until then (${attention})"
  kinds/command.sh: |
    # A command that must be on PATH, at the version if the line gives one (found in the output
    # of `<name> --version`). install cannot know this system's package manager, so it names the
    # command for a person to install, and fails.
    # A kind defines check and install, each called with the line's name, version and source.
    check() { command -v "$1" >/dev/null && { [ -z "$2" ] || "$1" --version 2>&1 | grep -qF "$2"; }; }
    install() {
      echo "$1${2:+ $2} is not on PATH: install it, then continue the run" >&2
      return 1
    }
  requirements: |
    # What this project's environment needs, one resource per line, in order: a line may rely
    # on the lines above it. `run.sh check` reports each (develop's precheck runs it);
    # `run.sh apply` installs what is missing and checks it again (the infra machine runs it,
    # and so can a fresh checkout or CI). Each kind is kinds/<kind>.sh. Another machine may
    # keep its own requirements beside its scripts; run.sh checks them after these. An edit
    # here is kept by `decree export infra`, or by the next run's `commit`.
    #
    # The AI CLI the official plugins' ai.sh picks (AI_CLI, else the first of claude, opencode
    # and copilot on PATH) belongs here too, such as `command   claude`.
    #
    # kind    name   version   source
    command   claude
    command   git
    command   cargo
  run.sh: |
    #!/usr/bin/env bash
    # Check or apply each line of every machine's requirements, as Ansible applies a play:
    # `check` reports each resource ok or missing and changes nothing; `apply` installs a
    # missing one and checks it again (changed), and stops at the first that fails, naming it.
    # The requirements are one list: infra's own first, then each other
    # .decree/machines/<machine>/scripts/requirements in name order. Each line reported ends with the
    # file it came from. A kind is kinds/<kind>.sh, beside this script.
    # Usage: run.sh check|apply
    set -euo pipefail
    here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
    machines=$(dirname "$(dirname "${here}")")
    mode=${1:?usage: run.sh check|apply}
    case ${mode} in check | apply) ;; *) echo "usage: run.sh check|apply" >&2; exit 2 ;; esac
    # One stream, so a failure prints after the lines before it.
    exec 2>&1

    files=()
    [ -f "${here}/requirements" ] && files+=("${here}/requirements")
    for f in "${machines}"/*/scripts/requirements; do
      [ -f "${f}" ] && [ "${f}" != "${here}/requirements" ] && files+=("${f}")
    done

    missing=0
    for file in "${files[@]}"; do
      from="machines/${file#"${machines}/"}"
      while read -r kind name version source; do
        [[ -z ${kind} || ${kind} == \#* ]] && continue
        label="${kind} ${name}${version:+ ${version}}"
        [ -f "${here}/kinds/${kind}.sh" ] || {
          echo "FAILED   ${label}: no kinds/${kind}.sh (${from})" >&2
          exit 1
        }
        # A subshell keeps one kind's functions from the next.
        # shellcheck source=/dev/null
        if (. "${here}/kinds/${kind}.sh" && check "${name}" "${version}" "${source}"); then
          echo "ok       ${label} (${from})"
        elif [ "${mode}" = check ]; then
          echo "missing  ${label} (${from})"
          missing=1
        elif (. "${here}/kinds/${kind}.sh" && install "${name}" "${version}" "${source}" &&
          check "${name}" "${version}" "${source}"); then
          echo "changed  ${label} (${from})"
        else
          echo "FAILED   ${label} (${from})" >&2
          exit 1
        fi
      done < "${file}"
    done
    exit "${missing}"
---
# infra

Declares what your project's environment needs (commands on `PATH`, and any other kind you add) in one file, and installs or checks it. An AI agent edits the declaration to meet a message; then every line is checked, and what is missing is installed where it can be.

## What it does

1. `clean_tree` (on entry): the run starts from a clean git work tree.
2. `declare`: the agent changes `.decree/machines/infra/scripts/requirements` to meet the message, and installs nothing. It writes `STOP` instead of guessing.
3. `apply`: `.decree/machines/infra/scripts/run.sh apply` checks each line of every `.decree/machines/*/scripts/requirements`, installs what is missing, and checks it again. A resource only a person can install (a `command` line, say) fails the run, naming it.
4. `done`: `commit` exports the edited plugin, so the new requirement lines reach `.decree/plugins/infra.md`, and commits. `failed`: `keep` leaves the changes for a retry.

`develop`'s `precheck` runs `run.sh check`, so a run fails fast when the environment is short of something.

## What it needs

- git, and an AI CLI on `PATH` (`claude`, `opencode` or `copilot`; see `AI_CLI`).

## How to use it

- Edit `.decree/machines/infra/scripts/requirements` by hand, one resource per line (`kind name version source`), or send a message: `echo "We need jq and node 22" | decree emit --machine infra && decree daemon --once`.
- A kind is `.decree/machines/infra/scripts/kinds/<kind>.sh`, defining `check` and `install`; `command` is the one shipped.
- `.decree/machines/infra/scripts/run.sh check` (or `apply`) works outside decree too, in CI or a fresh checkout.
- Edits are kept in this file by `decree export infra`, or by the next run's `commit`.
