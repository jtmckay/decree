---
name: decree_develop_by_size
needs:
- decree_ai
- decree_gliner_router
machine:
  description: Size up a change with a local classifier, then implement and test it, locally first or with Claude only, moving to Claude when local attempts fail the tests.
  concurrency_group: worktree
  initial: describe
  states:
    describe:
      invoke: describe
      transitions:
        done: size_up
    size_up:
      invoke:
        model:
          question: How much reasoning does this change need?
          router: decree_gliner_router
          min_confidence: 0.7
          output: describe
      transitions:
        small:
          target: local_first
          description: 'A local, mechanical change: a typo, a rename, a config value, one small function with a clear spec.'
        large:
          target: claude_only
          description: Design, several files, unclear requirements, concurrency or security.
        unsure: claude_only
        error: claude_only
    local_first:
      invoke:
        script:
          name: implement
          attempts:
          - local
          - local
          - claude
      transitions:
        done: done
        stop: failed
    claude_only:
      invoke:
        script:
          name: implement
          attempts:
          - claude
          - claude
      transitions:
        done: done
        stop: failed
    done:
      final: true
    failed:
      final: true
scripts:
  describe.sh: |
    #!/usr/bin/env bash
    # develop_by_size's describe: prints what the classifier reads, which is the
    # message's title, its acceptance criteria, and the files it names that exist
    # here with their line counts, so the classifier sees a size, not just prose.
    set -euo pipefail
    msg="$DECREE_MESSAGE"
    grep -m1 '^# ' "$msg" || true
    echo
    # the Acceptance Criteria section, up to the next ## heading
    awk '/^## / { on = ($0 ~ /^## Acceptance Criteria/) } on' "$msg"
    echo
    echo "Files it names (lines, path):"
    files=0
    lines=0
    while read -r path; do
      if [ -f "$path" ]; then
        n=$(wc -l < "$path")
        printf '%6d %s\n' "$n" "$path"
        files=$((files + 1))
        lines=$((lines + n))
      fi
    done < <(grep -oE '[A-Za-z0-9_./-]+\.[A-Za-z0-9]+' "$msg" | sort -u || true)
    echo "Total: $lines lines in $files files"
  implement.sh: |
    #!/usr/bin/env bash
    # develop_by_size's implement: one attempt. $DECREE_ATTEMPT_VALUE names the
    # tool, then the tests run: an attempt succeeds when the change passes them, so
    # a failing attempt exits non-zero and decree runs the next value. Both go through
    # ai.sh's `ai`, with AI_CLI naming the backend.
    #   local   a model served by Ollama, through OpenCode (`opencode run -m
    #           ollama/<model>`, https://opencode.ai/docs/cli/); the `ollama`
    #           provider and this model must be in opencode.json, as the example's
    #           README shows (https://opencode.ai/docs/providers/).
    #   claude  Claude, in small steps noted in progress.md, so a later attempt
    #           continues where this one stopped. If the message is unclear, it
    #           writes the question to STOP instead of guessing; the state's event
    #           is then `stop`, and the run fails until a person answers it,
    #           deletes STOP and runs `decree retry`.
    set -euo pipefail
    . ".decree/machines/${DECREE_NEED_DECREE_AI:-decree_ai}/scripts/ai.sh"
    OLLAMA_MODEL="${OLLAMA_MODEL:-qwen3-coder:30b}"
    TEST_CMD="${TEST_CMD:-cargo test}"
    progress="${DECREE_RUN_DIR}/progress.md"
    stop="${DECREE_RUN_DIR}/STOP"
    stopped() {
      [ -f "${stop}" ] || return 1
      cat "${stop}" >&2
      echo stop > "${DECREE_EVENT_FILE}"
    }
    stopped && exit 0

    case "${DECREE_ATTEMPT_VALUE:-}" in
      local)
        prompt="Read ${DECREE_MESSAGE} and implement it. It is a small, mechanical change:
    change only what it asks for, and keep the tests passing.
    Write scratch files only under ${DECREE_RUN_DIR}, never outside the project (not /tmp)."
        echo "=== opencode, ollama/${OLLAMA_MODEL} ==="
        echo "${prompt}"
        AI_CLI=opencode OPENCODE_MODEL="ollama/${OLLAMA_MODEL}" ai "${prompt}"
        ;;
      claude)
        prompt="Read ${DECREE_MESSAGE} and implement all requirements with proper error
    handling and tests. Work in small steps, one requirement at a time, and keep the
    code compiling between steps. After each step, append a line to ${progress}:
    what is done and what is next. If ${progress} exists, an earlier attempt was
    cut short: check the code against it and continue from there.
    If the message is ambiguous or conflicts with the code, do not guess: write
    the question to ${stop} and stop.
    Logs of earlier attempts, a local model's included, are in ${DECREE_RUN_DIR}.
    Write scratch files only under ${DECREE_RUN_DIR}, never outside the project (not /tmp)."
        echo "=== claude ==="
        echo "${prompt}"
        AI_CLI=claude ai "${prompt}"
        stopped && exit 0
        ;;
      *)
        echo "implement: unknown attempt value '${DECREE_ATTEMPT_VALUE:-}'; use local or claude" >&2
        exit 2
        ;;
    esac

    echo "=== ${TEST_CMD} ==="
    ${TEST_CMD}
    echo "tests pass with ${DECREE_ATTEMPT_VALUE}"
---
# decree_develop_by_size

Sends each change to the cheapest model that can do it. A local classifier (through `decree_gliner_router`) reads the change and its size, and decides whether a local model served by Ollama can do it, or it needs Claude. Each attempt implements the change and runs the tests; a failed attempt moves to the next, so local work that fails the tests goes to Claude in the same state.

## What it does

1. `describe` prints what the classifier reads: the message's title, its acceptance criteria, and the files it names, with their line counts.
2. `size_up` asks `decree_gliner_router` "How much reasoning does this change need?": `small` goes to `local_first`, `large` to `claude_only`. Below `min_confidence: 0.7` the event is `unsure`, which goes to Claude; so does `error`, when the classifier is not running.
3. `local_first` tries `local`, `local`, then `claude`; `claude_only` tries `claude` twice. Each attempt runs `implement.sh`, which asks through `ai.sh` (`local` is OpenCode with `OPENCODE_MODEL=ollama/<model>`, `claude` is Claude Code), then runs `$TEST_CMD` (`cargo test` by default) and fails if the tests fail. A `STOP` from any attempt ends the run in `failed` until a person answers it.

## What it needs

- The `decree_gliner_router` plugin and its classifier server, which `decree install decree_develop_by_size` adds with it.
- Ollama with a coding model (`OLLAMA_MODEL`, `qwen3-coder:30b` by default: `ollama pull qwen3-coder:30b`), and OpenCode configured with Ollama as a provider in the project's `opencode.json`.
- Claude Code (`claude`) on `PATH`.

## How to use it

Write an order file in `.decree/machines/decree_develop_by_size/order/`, then run `decree daemon --once`. To have a local model judge the size instead of the classifier, add `decree_llm_router` and point `size_up`'s `router:` at it. decree's repository has a demo message and the routing walkthrough (tuning `min_confidence` from the decision events) in `examples/route-by-complexity/`.
