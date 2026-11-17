---
name: qa
needs:
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
  attack.sh: |
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
  qa.sh: |
    # qa's settings and its guard, which qa's scripts source.
    #
    # Where the chaos tests live, apart from the project's other tests.
    QA_DIR=tests/chaos
    # The command that runs the chaos suite and exits non-zero if a test fails. qa does not
    # start until it is set. Once qa has found bugs, run it in develop's gate too, so a
    # fixed bug whose test is still marked expected to fail is caught. Keep an edit to this
    # file with `decree plugin export qa`, or let the next run's `commit` export it.
    QA_RUN="cargo test --test chaos"
    # QA_RUN="python3 -m unittest discover -s ${QA_DIR} -t . -v"
    # QA_RUN="npx playwright test ${QA_DIR}"
    # QA_RUN="cargo test --test chaos"
    # QA_RUN="go test ./${QA_DIR}/..."

    # Fail unless QA_RUN is set and this is a git work tree (the guard needs git).
    qa_ready() {
      if [ -z "${QA_RUN}" ]; then
        echo "qa: no chaos suite command; set QA_RUN in .decree/scripts/qa/qa.sh, then keep it with \`decree plugin export qa\`" >&2
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
    . .decree/scripts/qa/qa.sh
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
    # fails until a person answers it in plan.md, deletes STOP and runs `decree process --retry`.
    set -euo pipefail
    . .decree/scripts/qa/ai.sh
    . .decree/scripts/qa/qa.sh

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
    .decree/scripts/qa/invariants.md, and ${plan} if it exists. For each failing test in
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

1. `recon`: lists the attack surface, from the docs and the code, in the run's `surface.md`. It fails first if `QA_RUN` in `.decree/scripts/qa/qa.sh` is empty.
2. `attack`: writes chaos tests in `QA_DIR`, each naming the wrench (`wrenches.md`) and the invariant (`invariants.md`) it tests.
3. `run`: runs the chaos suite, `QA_RUN`, into `chaos.log`; a failing test is a finding, not an error.
4. `triage`: one bug report per root cause, its tests marked as expected to fail. It writes `STOP` instead of guessing.
5. `report`: one `develop` message per bug report.
6. `done` commits (exporting the edited plugin first); `failed` keeps the changes for a retry.

## What it needs

- The `develop` plugin (and so `infra`), which `decree plugin add qa` adds with it.
- git, and an AI CLI on `PATH` (`claude`, `opencode` or `copilot`; see `AI_CLI`).

## How to use it

1. Set `QA_DIR` and `QA_RUN` in `.decree/scripts/qa/qa.sh`: where the chaos tests go, and the command that runs them (`cargo test --test chaos`, say).
2. Write your project's invariants in `.decree/scripts/qa/invariants.md`, and adjust `wrenches.md`.
3. `echo "Attack the upload API" | decree emit --machine qa && decree process`.

Edits are kept in this file by `decree plugin export qa`, or by the next run's `commit`.
