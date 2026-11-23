---
name: decree_develop_react
extends: decree_develop
scripts:
  conventions.md: |
    # React conventions (develop_react's implement, fix and verify prompts read this)

    - Use the package manager the lockfile names (`package-lock.json`: npm, `pnpm-lock.yaml`:
      pnpm, `yarn.lock`: yarn; npm without one), and keep its lockfile in step with
      `package.json`. Never add a second lockfile.
    - Component tests with the project's own test runner (the one its `test` script runs,
      such as Vitest or Jest) and Testing Library (`@testing-library/react`, with
      `@testing-library/user-event` for interaction): render the component, find elements by
      role, label or text as a user would, and assert what the user sees. No snapshot-only
      tests, no tests of implementation details.
    - Test files beside the component (`Button.test.tsx` for `Button.tsx`) unless the project
      keeps them elsewhere; add no other runner.
    - Function components only, no classes; keep the `lint` and `typecheck` scripts clean where
      `package.json` defines them.
    - No new dependency unless the message allows it.
    - The project's `test` and `build` scripts must pass before you end; run the narrowest
      test first while working.
  gate.sh: |
    #!/usr/bin/env bash
    # develop_react's gate: develop_node's checks, then `build`, which a React project must
    # define; also kept in gate.log for fix. It uses the package manager the lockfile names
    # (package-lock.json: npm, pnpm-lock.yaml: pnpm, yarn.lock: yarn; npm without one),
    # installs the dependencies as that manager does for CI, then runs `lint`, `typecheck`
    # and `test`, each only if package.json defines it, and says which it skipped, then
    # `build`. It names `pass` if they all pass and `fail` if one fails or there is no
    # `build`; a non-zero exit, such as a missing tool, means they could not run, and ends
    # the run without fix. decree_develop_react is never edited: a plugin of
    # your own that extends it holds a gate.sh with your project's own checks.
    set -euo pipefail
    log="${DECREE_ROOT_RUN_DIR}/gate.log"

    # The scripts run if package.json defines them, in order; then those it must define.
    optional=(lint typecheck test)
    required=(build)

    # Its arguments, as `a, b, c`.
    list() {
      local joined
      printf -v joined '%s, ' "$@"
      echo "${joined%, }"
    }

    # The checks cannot run.
    cannot() {
      echo "gate: $1; the checks cannot run" | tee "${log}" >&2
      exit 1
    }
    [ -f package.json ] || cannot "no package.json in $(pwd)"
    command -v awk >/dev/null || cannot "awk not found on PATH"

    # The package manager, and how it installs for CI.
    if [ -f package-lock.json ]; then
      pm=npm install=(npm ci)
    elif [ -f pnpm-lock.yaml ]; then
      pm=pnpm install=(pnpm install --frozen-lockfile)
    elif [ -f yarn.lock ]; then
      pm=yarn install=(yarn install --frozen-lockfile)
      # Yarn 2 and later (configured in .yarnrc.yml) spell it --immutable.
      [ ! -f .yarnrc.yml ] || install=(yarn install --immutable)
    else
      pm=npm install=(npm install)
    fi
    command -v "${pm}" >/dev/null || cannot "${pm} not found on PATH"

    # The names of the scripts package.json defines: the keys of its top-level "scripts"
    # object, one per line. A JSON scanner in awk, so the gate needs no node to read it.
    defined=$(awk '
      BEGIN { RS = "\001" }
      {
        n = length($0); depth = 0; key = ""; in_scripts = 0
        for (i = 1; i <= n; i++) {
          c = substr($0, i, 1)
          if (c == "\"") {
            s = ""
            for (i++; i <= n; i++) {
              c = substr($0, i, 1)
              if (c == "\\") { s = s c substr($0, i + 1, 1); i++; continue }
              if (c == "\"") break
              s = s c
            }
            k = i + 1
            while (k <= n && substr($0, k, 1) ~ /[ \t\r\n]/) k++
            if (substr($0, k, 1) == ":") {
              if (depth == 1) key = s
              else if (depth == 2 && in_scripts) print s
            }
          } else if (c == "{" || c == "[") {
            depth++
            if (depth == 2 && c == "{" && key == "scripts") in_scripts = 1
          } else if (c == "}" || c == "]") {
            if (depth == 2) in_scripts = 0
            if (depth == 2) key = ""
            depth--
          } else if (c == "," && depth == 1) {
            key = ""
          }
        }
      }' package.json) || cannot "package.json could not be read"
    has() { grep -qxF -- "$1" <<<"${defined}"; }

    run=()
    skipped=()
    for script in "${optional[@]}"; do
      if has "${script}"; then run+=("${script}"); else skipped+=("${script}"); fi
    done
    run+=("${required[@]}")

    if [ "${#run[@]}" -eq 0 ]; then
      echo "gate: package.json defines none of $(list "${optional[@]}"): checked nothing" | tee "${log}"
      echo pass > "${DECREE_EVENT_FILE}"
      exit 0
    fi

    # The checks: install, then each script; the first that fails names `fail`.
    checks() {
      if [ "${#skipped[@]}" -gt 0 ]; then
        echo "gate: skipped $(list "${skipped[@]}"): not in package.json's scripts"
      fi
      "${install[@]}" || return 1
      for script in "${run[@]}"; do
        if ! has "${script}"; then
          echo "gate: package.json defines no \`${script}\` script, which this gate needs"
          return 1
        fi
        "${pm}" run "${script}" || return 1
      done
    }

    if checks 2>&1 | tee "${log}"; then
      echo pass > "${DECREE_EVENT_FILE}"
    else
      echo fail > "${DECREE_EVENT_FILE}"
    fi
  requirements: |
    # What develop_react's gate needs, one resource per line, as in infra's requirements:
    # infra's `run.sh check` (develop_react's precheck) reports each, and `run.sh apply`
    # installs what is missing. A project using pnpm or yarn adds that line, such as
    # `command   pnpm`.
    #
    # kind    name   version   source
    command   node
    command   npm
---
# decree_develop_react

[`decree_develop`](decree_develop.md) for a React project. It extends `decree_develop` (`extends: decree_develop`) and holds only three files: its `gate.sh`, its `conventions.md` and its `requirements`; its machine and every other script are `decree_develop`'s, installed from it. So it has a gate that runs [`decree_develop_node`](decree_develop_node.md)'s checks and then `build`, and prompts that ask for component tests with the project's own test runner and Testing Library. Implements a message, such as an order file describing a change, with an AI coding agent, then runs the gate and has the agent check the message's acceptance criteria. Each run ends in one git commit, or in `failed` with its changes left in the work tree.

## What it does

As `decree_develop` does (`clean_tree`, `precheck`, `implement`, `gate`, `fix`, `final_gate`, `verify`, then `commit` or `keep`), except:

- `gate` runs `decree_develop_node`'s checks (the lockfile's package manager installs for CI, then `lint`, `typecheck` and `test`, each only if `package.json` defines it), then `build`, which a React project must define: without a `build` script it names `fail`. Without `package.json`, or without the package manager on `PATH`, it exits non-zero, naming what is missing, and the run ends without `fix`.
- `implement`, `fix` and `verify` tell the agent to follow its `conventions.md`: component tests with the project's own test runner and Testing Library (`@testing-library/react`), finding elements as a user would.

## What it needs

- git, and a project that is a git work tree with at least one commit.
- An AI CLI on `PATH`: `claude`, `opencode` or `copilot`, picked as `decree_develop` picks it.
- `node` and `npm`, which its `requirements` declares, so `precheck` fails fast without them. A project using pnpm or yarn adds that line (`command   pnpm`).
- `decree_develop`, which it extends, and what that needs (`decree_ai`, `decree_infra`, `decree_order`): `decree install decree_develop_react` adds them with it, and installs both `machines/decree_develop/` and `machines/decree_develop_react/`.

## How to use it

1. `decree install decree_develop_react`. `decree_develop_react` is never edited: to add the project's own checks or rules, write a plugin of your own that extends it (`name: app_develop`, `extends: decree_develop_react`) holding its own `scripts/gate.sh` or `scripts/conventions.md`, and send messages to that. A project with a React front end and a Rust or Node back end adds both variants and names one in each message.
2. Write an order file, `.decree/machines/decree_develop_react/order/A01-<what>.md`, saying what to change and how to tell it is done.
3. Run `decree daemon --once`, or keep `decree daemon` running: the official `decree_order` plugin runs the file at start.
