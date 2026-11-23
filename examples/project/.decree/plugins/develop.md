---
name: develop
needs:
- decree_ai
machine:
  description: Implement a message, then test it.
  initial: implement
  states:
    implement:
      invoke:
        script:
          name: implement
          attempts:
          - local
          - claude
      transitions:
        done: test
    test:
      invoke: test
      transitions:
        done: done
    done:
      final: true
    failed:
      final: true
scripts:
  implement.sh: |
    #!/usr/bin/env bash
    # develop's implement, a stand-in for an AI agent: machines/develop/scripts/ is checked
    # before scripts/. The machine's attempt list says which model does this
    # attempt, in $DECREE_ATTEMPT_VALUE: a choice of how, not of what runs next.
    # `local` is a model served by Ollama through OpenCode; `claude` is Claude. Both go
    # through ai.sh's `ai`, which AI_CLI points at a backend.
    # A non-zero exit is error; decree then runs the next attempt in place.
    set -euo pipefail
    . ".decree/machines/${DECREE_NEED_DECREE_AI:-decree_ai}/scripts/ai.sh"
    prompt="Read $DECREE_MESSAGE and make the change it asks for."
    case "$DECREE_ATTEMPT_VALUE" in
      claude) AI_CLI=claude ai "$prompt" ;;
      *)      AI_CLI=opencode OPENCODE_MODEL="${OPENCODE_MODEL:-ollama/qwen3:8b}" ai "$prompt" ;;
    esac
  test.sh: |
    #!/usr/bin/env bash
    # develop's test. Exit 0 is done; anything else is error, which goes to
    # failed, implicitly.
    set -euo pipefail
    cargo test
cron:
  nightly-audit.md: |
    ---
    cron: "0 3 * * *"
    machine: develop
    ---
    # No dependency has an advisory that a patched version fixes

    ## Overview

    A vulnerability whose fix is published should not stay in the build overnight.

    ## Outcome

    `cargo audit` reports no advisory that a compatible patched version of the dependency fixes.

    ## Constraints

    - The project's own code does not change, only the versions of its dependencies.

    ## Acceptance Criteria

    - **Given** the project after the run
      **When** `cargo audit` runs
      **Then** it lists no advisory that a compatible patched version fixes
---
# develop

Implement a message, then test it.
