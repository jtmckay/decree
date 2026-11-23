---
name: decree_develop_rust
extends: decree_develop
scripts:
  conventions.md: |
    # Rust conventions (develop_rust's implement, fix and verify prompts read this)

    - Format with `cargo fmt`; keep `cargo clippy --all-targets -- -D warnings` clean.
    - Unit tests in a `#[cfg(test)] mod tests` beside the code; behaviour a user sees
      (commands, files, exit codes) in `tests/`, driving the built binary.
    - Return errors with context; no `unwrap()` or `expect()` outside tests unless the
      invariant is proven on the line above.
    - No new dependency unless the message allows it.
    - `cargo test` must pass before you end; run the narrowest test first while working.
  gate.sh: |
    #!/usr/bin/env bash
    # develop_rust's gate: cargo's standard checks, also kept in gate.log for fix. It
    # names `pass` if they all pass and `fail` if one fails; a non-zero exit, such
    # as a missing tool, means they could not run, and ends the run without fix.
    # decree_develop_rust is never edited: to add your project\'s own checks, extend it
    # with a plugin of your own holding its own scripts/gate.sh.
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
  requirements: |
    # What develop_rust's gate needs, one resource per line, as in infra's requirements:
    # infra's `run.sh check` (develop_rust's precheck) reports each, and `run.sh apply`
    # installs what is missing.
    #
    # kind    name   version   source
    command   cargo
---
# decree_develop_rust

[`decree_develop`](decree_develop.md) for a Rust project. It extends `decree_develop` (`extends: decree_develop`) and holds only three files: its `gate.sh`, its `conventions.md` and its `requirements`; its machine and every other script are `decree_develop`'s, installed from it. So it has a gate that already runs cargo's standard checks and prompts that follow Rust conventions. Implements a message, such as an order file describing a change, with an AI coding agent, then runs the gate and has the agent check the message's acceptance criteria. Each run ends in one git commit, or in `failed` with its changes left in the work tree.

## What it does

As `decree_develop` does (`clean_tree`, `precheck`, `implement`, `gate`, `fix`, `final_gate`, `verify`, then `commit` or `keep`), except:

- `gate` runs `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test`, in that order, and names `fail` at the first that fails. Without `cargo` on `PATH` it exits non-zero, naming it, and the run ends without `fix`.
- `implement`, `fix` and `verify` tell the agent to follow its `conventions.md`: unit tests beside the code, behaviour a user sees in `tests/`, errors with context, no new dependency unless the message allows it.

## What it needs

- git, and a project that is a git work tree with at least one commit.
- An AI CLI on `PATH`: `claude`, `opencode` or `copilot`, picked as `decree_develop` picks it.
- `cargo`, which its `requirements` declares, so `precheck` fails fast without it.
- `decree_develop`, which it extends, and what that needs (`decree_ai`, `decree_infra`, `decree_order`): `decree install decree_develop_rust` adds them with it, and installs both `machines/decree_develop/` and `machines/decree_develop_rust/`.

## How to use it

1. `decree install decree_develop_rust`. `decree_develop_rust` is never edited: to add the project's own checks or rules, write a plugin of your own that extends it (`name: app_develop`, `extends: decree_develop_rust`) holding its own `scripts/gate.sh` or `scripts/conventions.md`, and send messages to that.
2. Write an order file, `.decree/machines/decree_develop_rust/order/A01-<what>.md`, saying what to change and how to tell it is done.
3. Run `decree daemon --once`, or keep `decree daemon` running: the official `decree_order` plugin runs the file at start.
