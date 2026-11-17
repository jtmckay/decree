---
name: develop
needs:
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
      - restart
    failed:
      final: true
      onentry:
      - keep
scripts:
  ai.sh: |
    # The AI helper the official plugins' scripts source, a copy in each one's scripts
    # directory, by its path from the project root (`. .decree/scripts/develop/ai.sh`); then
    # `ai "<prompt>"` asks the AI and prints its reply.
    #
    # Every backend is a function here: ai_claude, ai_opencode, ai_copilot. `ai` calls,
    # most specific first: the one the run's data entry `ai_cli` names
    # (DECREE_DATA_AI_CLI: a message's or a parent's `params`, else the machine's
    # default), so a child machine invoked with `params: { ai_cli: claude }` works with
    # Claude whatever the project's setting; else the one AI_CLI names (in .decree/.env,
    # or the shell decree runs in); else the first of claude, opencode and copilot found
    # on PATH. To use another backend, add an ai_<name> function and set AI_CLI=<name>.
    #
    # The agent names no event (it writes STOP instead): a script it runs, such as the
    # gate, gets DECREE_EVENT_FILE=/dev/null, and the caller's event file is emptied
    # after the call, in case the agent found its path anyway. A call that leaves a STOP
    # file in the root run's folder ($DECREE_ROOT_RUN_DIR) returns 0 whatever the
    # backend's exit code, so the caller's STOP check names `stop` instead of failing
    # an attempt.
    #
    # Every call adds a line to .decree/ai-use.md (git ignores it): when, which run and
    # state, which CLI and model, the run's attempt and the step's, how long it took
    # and its exit code.

    # The backend `ai` calls: DECREE_DATA_AI_CLI, else AI_CLI, else the first of claude,
    # opencode and copilot on PATH. Prints nothing and fails if there is none.
    pick_ai_cli() {
      local cli=${DECREE_DATA_AI_CLI:-${AI_CLI:-}}
      if [ -z "${cli}" ]; then
        for cli in claude opencode copilot; do
          command -v "${cli}" >/dev/null && break
          cli=
        done
      fi
      [ -n "${cli}" ] && echo "${cli}"
    }

    ai() {
      local cli
      cli=$(pick_ai_cli) || {
        echo "ai: no AI CLI: set AI_CLI (in .decree/.env) or put claude, opencode or copilot on PATH" >&2
        return 2
      }
      if ! declare -F "ai_${cli}" >/dev/null; then
        echo "ai: no ai_${cli} function in ai.sh (AI_CLI=${cli})" >&2
        return 2
      fi
      local caller_event=${DECREE_EVENT_FILE:-} status=0 start=${SECONDS}
      local -x DECREE_EVENT_FILE=/dev/null
      "ai_${cli}" "$1" || status=$?
      [ -z "${caller_event}" ] || : > "${caller_event}"
      ai_log "${cli}" "$((SECONDS - start))" "${status}" || true
      [ ! -f "${DECREE_ROOT_RUN_DIR:-${DECREE_RUN_DIR:-.}}/STOP" ] || status=0
      return "${status}"
    }

    # One line in .decree/ai-use.md, its table header first. Past 2 MiB it keeps the
    # header and the last 10000 calls.
    ai_log() {
      local log="${DECREE_PROJECT_ROOT:-.}/.decree/ai-use.md" model
      [ -d "${log%/*}" ] || return 0
      case $1 in
        claude) model=${CLAUDE_MODEL:-default} ;;
        opencode) model=${OPENCODE_MODEL:-default} ;;
        *) model=default ;;
      esac
      if [ ! -s "${log}" ]; then
        printf '| time | run | state | cli | model | run attempt | attempt | seconds | exit |\n' > "${log}"
        printf '| --- | --- | --- | --- | --- | --- | --- | --- | --- |\n' >> "${log}"
      fi
      printf '| %s | %s | %s/%s | %s | %s | %s | %s | %s | %s |\n' \
        "$(date '+%Y-%m-%d %H:%M:%S')" "${DECREE_MESSAGE_ID:-}" "${DECREE_MACHINE:-}" \
        "${DECREE_STATE:-}" "$1" "${model}" "${DECREE_RUN_ATTEMPT:-1}" "${DECREE_ATTEMPT:-1}" \
        "$2" "$3" >> "${log}"
      if [ "$(wc -c < "${log}")" -gt 2097152 ]; then
        { head -n 2 "${log}"; tail -n 10000 "${log}"; } > "${log}.tmp" && mv "${log}.tmp" "${log}"
      fi
    }

    # --- Claude ---------------------------------------------------------------------
    # Each call is a new session, listed in the run's sessions.txt with its transcript,
    # which Claude writes as it goes and keeps even if the session dies.
    # When Claude stops at its usage limit (its output names a "usage limit" and a
    # "reset"), wait until the reset time it names, or an hour if it names none, then
    # resume the same session with the same prompt: the work it did is kept, and the
    # wait uses none of the step's attempts. A reset further away than AI_LIMIT_WAIT
    # (default 6h; s, m, h or d) is not waited for: the call writes STOP saying when to
    # retry, so the run stops instead of sleeping through a weekly limit.
    # CLAUDE_PERMISSION_MODE is passed to every call, so the run does not inherit your
    # interactive default (in `plan` mode Claude can only plan, and changes nothing). `auto`
    # approves what Claude's classifier judges safe; `acceptEdits` approves only file edits.
    # CLAUDE_MODEL, if set (haiku, sonnet, a full model id), picks the model for every call.
    CLAUDE_PERMISSION_MODE="${CLAUDE_PERMISSION_MODE:-auto}"
    AI_LIMIT_WAIT="${AI_LIMIT_WAIT:-6h}"
    ai_claude() {
      local prompt=$1 session out status wait
      session=$(new_session_id)
      echo "=== claude session ${session} ===" >&2
      echo "${DECREE_STATE} ${session} ${HOME}/.claude/projects/$(pwd | sed 's/[^A-Za-z0-9]/-/g')/${session}.jsonl" \
        >> "${DECREE_RUN_DIR}/sessions.txt"
      local session_flag=(--session-id "${session}")
      while true; do
        out=$(mktemp)
        # stdout to stdout and stderr to stderr, both also kept in $out
        if { claude -p --permission-mode "${CLAUDE_PERMISSION_MODE}" ${CLAUDE_MODEL:+--model "${CLAUDE_MODEL}"} "${session_flag[@]}" "${prompt}" 2>&1 1>&3 3>&- | tee -a "${out}" >&2; } 3>&1 | tee -a "${out}"; then
          status=0
        else
          status=$?
        fi
        if [ "${status}" -eq 0 ] || ! usage_limit "${out}"; then
          rm -f "${out}"
          return "${status}"
        fi
        wait=$(seconds_until_reset "${out}")
        rm -f "${out}"
        if [ "${wait}" -gt "$(seconds_of "${AI_LIMIT_WAIT}")" ]; then
          echo "[Claude token limit] Usage limit reached; it resets $(reset_note "${wait}" ""), more than AI_LIMIT_WAIT (${AI_LIMIT_WAIT}) away. Wrote STOP." >&2
          {
            echo "Claude's usage limit is reached and resets $(reset_note "${wait}" "")."
            echo "That is more than AI_LIMIT_WAIT (${AI_LIMIT_WAIT}) away, so the run stopped instead of waiting."
            echo "After the reset, delete this file and run \`decree process --retry ${DECREE_MESSAGE_ID:-<id>}\`."
          } > "${DECREE_ROOT_RUN_DIR:-${DECREE_RUN_DIR}}/STOP"
          return 1
        fi
        reset_note "${wait}" "Usage limit reached." >&2
        sleep "${wait}"
        echo "[Claude token limit] Resuming session ${session}" >&2
        session_flag=(--resume "${session}")
      done
    }

    # A random version 4 UUID, the form `claude --session-id` takes.
    new_session_id() {
      printf '%04x%04x-%04x-4%03x-%04x-%04x%04x%04x\n' "${RANDOM}" "${RANDOM}" "${RANDOM}" \
        $((RANDOM & 0xfff)) $((RANDOM & 0x3fff | 0x8000)) "${RANDOM}" "${RANDOM}" "${RANDOM}"
    }

    # Claude's wording varies: "usage limit reached … reset at 10:00 PM", "You've hit
    # your session limit · resets 1:10am (America/Denver)", weekly and 5-hour limits.
    usage_limit() {
      grep -qiE '(usage|session|weekly|daily|5-hour) limit|limit (reached|hit)' "$1" && grep -qi 'reset' "$1"
    }

    # Seconds until the time in "Limits reset at 10:00 PM" (local time, tomorrow if it
    # has passed today), or 3600 if there is no such time.
    seconds_until_reset() {
      local at h m now_h now_m now_s now wait=3600
      # "reset at 10:00 pm", "resets 1:10am", "resets 3am": read as local time.
      at=$(tr '[:upper:]' '[:lower:]' < "$1" | grep -oE 'resets?( +at)? +[0-9]{1,2}(:[0-9]{2})? *[ap]m' \
        | head -n 1 | grep -oE '[0-9]{1,2}(:[0-9]{2})? *[ap]m') || true
      IFS=: read -r now_h now_m now_s <<< "$(date +%H:%M:%S)"
      now=$((10#${now_h} * 3600 + 10#${now_m} * 60 + 10#${now_s}))
      if [ -n "${at}" ]; then
        h=${at%%[!0-9]*}
        h=$((10#${h}))
        m=0
        case ${at} in *:*) m=${at#*:}; m=$((10#${m:0:2})) ;; esac
        if [ "${h}" -le 12 ] && [ "${m}" -le 59 ]; then
          h=$((h % 12))
          case ${at} in *pm) h=$((h + 12)) ;; esac
          wait=$((h * 3600 + m * 60 - now))
          [ "${wait}" -gt 0 ] || wait=$((wait + 86400))
        fi
      fi
      echo "${wait}"
    }

    # "[Claude token limit] <what> Waiting until 22:00 (30m 0s) to retry." for a wait of
    # $1 seconds; with an empty <what>, just "at 22:00 (in 30m 0s)".
    reset_note() {
      local now_h now_m now_s now at
      IFS=: read -r now_h now_m now_s <<< "$(date +%H:%M:%S)"
      now=$((10#${now_h} * 3600 + 10#${now_m} * 60 + 10#${now_s}))
      at=$(((now + $1) % 86400))
      if [ -z "$2" ]; then
        printf 'at %02d:%02d (in %dm %ds)' $((at / 3600)) $((at % 3600 / 60)) $(($1 / 60)) $(($1 % 60))
      else
        printf '[Claude token limit] %s Waiting until %02d:%02d (%dm %ds) to retry.\n' \
          "$2" $((at / 3600)) $((at % 3600 / 60)) $(($1 / 60)) $(($1 % 60))
      fi
    }

    # `90`, `90s`, `10m`, `6h` or `1d` in seconds.
    seconds_of() {
      local n=${1%[smhd]}
      case $1 in
        *m) echo $((n * 60)) ;;
        *h) echo $((n * 3600)) ;;
        *d) echo $((n * 86400)) ;;
        *) echo "${n}" ;;
      esac
    }

    # --- OpenCode -------------------------------------------------------------------
    # OPENCODE_MODEL, if set (provider/model, e.g. llama.cpp/qwen3), picks the model.
    # OpenCode's file tools ask before touching a path outside the project, and `opencode
    # run` cannot answer, so with its default `"external_directory": "ask"` it rejects
    # the call and ends the session with no reply (exit 0). `decree init --permissions`
    # sets it to "deny", which rejects the call but lets the session go on. When a reject
    # ends the session anyway, the call continues it (`opencode run -c`) up to AI_NUDGES
    # times (default 2), telling the agent where scratch files go. The permission covers
    # OpenCode's file tools only, not shell commands: it is no sandbox, and the prompt is
    # what keeps writes in the project. The last call's output stays in the run folder,
    # as opencode.out.
    AI_NUDGES="${AI_NUDGES:-2}"
    ai_opencode() {
      local out="${DECREE_RUN_DIR}/opencode.out" nudge=0 status=0
      local model=()
      [ -z "${OPENCODE_MODEL:-}" ] || model=(-m "${OPENCODE_MODEL}")
      opencode_call "${out}" "${model[@]}" "$1" || status=$?
      while grep -q 'permission requested: .*auto-rejecting' "${out}" && [ "${nudge}" -lt "${AI_NUDGES}" ]; do
        nudge=$((nudge + 1))
        echo "=== opencode: a tool call was rejected; continuing the session (${nudge}/${AI_NUDGES}) ===" >&2
        status=0
        opencode_call "${out}" -c "${model[@]}" "A tool call you made was rejected because it \
    used a path outside the project, such as /tmp. Write scratch files only under \
    ${DECREE_RUN_DIR}. Continue the task from where you stopped and finish it exactly as \
    the first instructions say, including how to end your reply." || status=$?
      done
      return "${status}"
    }

    # `opencode run <args>`, its stdout to stdout and stderr to stderr, both also in $1.
    opencode_call() {
      local out=$1
      shift
      : > "${out}"
      { opencode run "$@" 2>&1 1>&3 3>&- | tee -a "${out}" >&2; } 3>&1 | tee -a "${out}"
    }

    # --- Copilot --------------------------------------------------------------------
    ai_copilot() {
      copilot -p "$1"
    }
  clean_tree.sh: |
    #!/usr/bin/env bash
    # Root onentry script: the run starts from a clean git work tree, so the commit at `done`
    # holds this run's work only, and a failed run's changes never mix into the next one.
    # It runs again when `decree process --retry` continues the run; by then the tree holds
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
        echo "  to continue it on them: \`decree process --retry ${id}\`"
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
      echo "if a failed run left them, its file in .decree/attention/ names it; continue it with \`decree process --retry <id>\`" >&2
      exit 1
    fi
    git rev-parse HEAD > "${mark}"
    echo "clean at $(git rev-parse --short HEAD)"
  commit.sh: |
    #!/usr/bin/env bash
    # onentry script of `done`: commit the run's work, titled with the message's first `# `
    # heading. For a migration, decree has already added it to processed.md, so the one commit
    # holds the work and the ledger line, and the title ends with the migration's file name
    # (`migration:` in the run's message.md), as its run id is a time id. Nothing changed is
    # nothing to commit. If the commit fails, the run ends in `failed`, not `done`. Only the
    # project folder is committed (`-- .`), even if something else in a larger repository is
    # staged.
    # First it exports every plugin whose installed copy differs from its plugin file
    # (`decree plugin export` with no name), so what the run changed in machines/ or scripts/
    # (an infra run's new requirement, a develop run's gate) reaches .decree/plugins/ and is in
    # this one commit; git ignores machines/ and scripts/. A failed run exports nothing: `keep`
    # leaves its edits in the installed copy, where a retry builds on them.
    set -euo pipefail
    "${DECREE}" plugin export
    git add -A -- .
    if git diff --cached --quiet -- .; then
      echo "nothing to commit"
      exit 0
    fi
    title=$(sed -n 's/^# //p' "${DECREE_MESSAGE}" | head -n 1)
    title=${title:-${DECREE_MACHINE}: ${DECREE_MESSAGE_ID}}
    migration=$(sed -n '2,/^---$/s/^migration: //p' "${DECREE_MESSAGE}" | head -n 1)
    if [ -n "${migration}" ]; then
      title="${title} (${migration})"
    fi
    git commit -q -m "${title}" \
      -m "decree run ${DECREE_MESSAGE_ID} (${DECREE_MACHINE})" -- .
    git log -1 --format='committed %h %s'
  conventions.md: |
    # Rust conventions (develop's implement, fix and verify prompts read this)

    - Format with `cargo fmt`; keep `cargo clippy --all-targets -- -D warnings` clean.
    - Unit tests in a `#[cfg(test)] mod tests` beside the code; behaviour a user sees
      (commands, files, exit codes) in `tests/`, driving the built binary.
    - Return errors with context; no `unwrap()` or `expect()` outside tests unless the
      invariant is proven on the line above.
    - No new dependency unless the message allows it.
    - `cargo test` must pass before you end; run the narrowest test first while working.
  fix.sh: |
    #!/usr/bin/env bash
    # develop's fix: the AI fixes what the gate reported. Like implement, it
    # writes STOP instead of guessing.
    set -euo pipefail
    . .decree/scripts/develop/ai.sh

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
    (.decree/scripts/develop/gate.sh) failed; its output is in
    ${DECREE_ROOT_RUN_DIR}/gate.log, and ${progress} notes what was done so far.
    Fix the failures, append a line to ${progress} for each fix, and run the
    gate again. If ${plan} exists, follow it. If a failure needs a decision the
    message does not make, write the question to ${stop} and stop.
    Follow the conventions in .decree/scripts/develop/conventions.md.
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
    # This repository's checks: cargo fmt, clippy and the tests, one per line
    # (tests/ci_test.rs reads them to keep .github/workflows/ci.yml the same).
    set -euo pipefail
    log="${DECREE_ROOT_RUN_DIR}/gate.log"

    # A tool the checks need is not on PATH: they cannot run.
    missing() {
      echo "gate: $1 not found on PATH; the checks cannot run" | tee "${log}" >&2
      exit 1
    }
    command -v cargo >/dev/null || missing cargo

    # The checks, as one && chain: the first that fails names `fail`.
    checks() {
      cargo fmt --check &&
        cargo clippy --all-targets -- -D warnings &&
        cargo test
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
    # `decree process --retry`. Its notes live in the root run's folder
    # ($DECREE_ROOT_RUN_DIR, the run's own unless it is a child run), so a child machine
    # whose steps are develop's scripts shares them.
    set -euo pipefail
    . .decree/scripts/develop/ai.sh

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
    Follow the conventions in .decree/scripts/develop/conventions.md.
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
    # on them. `decree process --retry` of this run removes both; to set the changes aside
    # instead, stash them and delete the run's attention file.
    # Only the project folder counts (`-- .`): a decree project can be a subfolder of a larger
    # repository. A run that failed because a person's own changes were there from the start
    # (`clean_tree` did not pass) leaves them alone and holds nothing.
    # Edits to an installed plugin (machines/, scripts/, which git ignores) count too: a failed
    # run exports nothing, so they stay in the installed copy, and the hold keeps the next run's
    # `commit` from exporting them (`decree plugin export --dry-run` names them).
    set -euo pipefail
    [ -f "${DECREE_RUN_DIR}/clean_tree" ] || exit 0
    changes=$(git status --porcelain -- .)
    edited=$("${DECREE}" plugin export --dry-run | sed -n 's/^would write //p')
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
      echo "to continue the run on them: \`decree process --retry ${DECREE_MESSAGE_ID}\`"
      echo "to set them aside instead: \`git stash push --include-untracked -m \"decree ${DECREE_MESSAGE_ID}\" -- .\` (and \`decree plugin install <name>\` for an installed copy), then delete ${attention}"
    } > "${DECREE_RUN_DIR}/hold"
    cat "${DECREE_RUN_DIR}/hold"
    echo "no other run starts until then (${attention})"
  precheck.sh: |
    #!/usr/bin/env bash
    # Fail fast if the environment every .decree/scripts/*/requirements declares is not all
    # there, such as the AI CLI or git missing from PATH. An infra message adds what is missing;
    # develop does not.
    set -euo pipefail
    .decree/scripts/infra/run.sh check
  restart.sh: |
    #!/usr/bin/env bash
    # onentry script of `done`, after commit: hold the queue once this run has
    # committed, so `decree process` exits and .decree/scripts/develop/run-queue.sh
    # builds decree from the new commit before the next migration runs. Each
    # migration here changes decree itself, so the next one must run on the result.
    # Only migrations: an inbox run is not part of the queue's rebuild loop.
    set -euo pipefail
    [ "${DECREE_TRIGGER:-}" = migration ] || exit 0
    "${DECREE}" stop -m "develop committed ${DECREE_MESSAGE_ID}: run-queue.sh rebuilds decree and goes on"
  run-queue.sh: |
    #!/usr/bin/env bash
    # run-queue: run this repository's pending migrations unattended, each one on a
    # decree built from the commit before it.
    #
    #   .decree/scripts/develop/run-queue.sh
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
  verify.sh: |
    #!/usr/bin/env bash
    # develop's verify: the AI checks the acceptance criteria and ends its reply
    # with `VERDICT: pass` or `VERDICT: fail`; the last such line is the event. A
    # reply without one fails the attempt, so decree asks again. Like implement, it
    # writes STOP instead of guessing. On `fail` it leaves gaps.md, what is missing, for
    # the next round or retry; an attempt that dies keeps the last one.
    set -euo pipefail
    . .decree/scripts/develop/ai.sh

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
    Follow the conventions in .decree/scripts/develop/conventions.md.
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

Implements a message, such as a migration describing a change, with an AI coding agent, then runs your project's checks and has the agent check the message's acceptance criteria. Each run ends in one git commit, or in `failed` with its changes left in the work tree.

## What it does

1. `clean_tree` (on entry): the run starts from a clean git work tree, so the commit at the end holds this run's work only.
2. `precheck`: fails fast if anything that `.decree/scripts/*/requirements` declares is missing, by running infra's `run.sh check` (so `develop` needs `infra`).
3. `implement`: the agent implements the message in small steps, logging each in the run's `progress.md`. If the message is ambiguous it writes `STOP` with its question instead of guessing, and the run ends in `failed`.
4. `gate`: your project's checks, `.decree/scripts/develop/gate.sh`. Passing goes to `verify`; failing goes to `fix`, where the agent fixes what `gate.log` reports, then to `final_gate`, whose failure ends the run.
5. `verify`: the agent checks every acceptance criterion and ends its reply with `VERDICT: pass` or `VERDICT: fail`.
6. `done`: `commit` exports every plugin a run edited (`decree plugin export`), then commits the run's work in one commit, titled with the message's first heading. `failed`: `keep` leaves the changes in the work tree for `decree process --retry <id>`, and holds the queue.

## What it needs

- git, and a project that is a git work tree with at least one commit.
- An AI CLI on `PATH`: `claude`, `opencode` or `copilot`. `ai.sh` uses the run's `ai_cli` data, else `AI_CLI` (in `.decree/.env`), else the first of them it finds.
- The `infra` plugin, which `decree plugin add develop` adds with it.

## How to use it

1. Put your project's checks in `.decree/scripts/develop/gate.sh` (it prints `gate: no checks configured` until you do), for instance `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`. The edit is kept in this file by `decree plugin export develop`, or by the next run's `commit`.
2. Write a migration, `.decree/migrations/01-<what>.md`, with `machine: develop` in its frontmatter, saying what to change and how to tell it is done.
3. Run `decree process`.
