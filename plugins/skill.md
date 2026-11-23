---
name: skill
machine:
  description: Keep the decree skill in the project, .claude/skills/decree/ or .github/skills/decree/, the one this decree ships, each time the daemon starts.
  initial: install
  states:
    install:
      description: Copy each skill file that differs into each AI CLI's skill folder the project has.
      invoke: install
      transitions:
        done: done
    done:
      final: true
    failed:
      final: true
scripts:
  install.sh: |
    #!/usr/bin/env bash
    # skill's install: copy the decree skill, this folder's skill/, into each AI CLI's skill
    # folder the project has, .claude/skills/decree/ and .github/skills/decree/; with neither,
    # into the one for AI_CLI (claude or copilot), else the first of claude and copilot on PATH.
    # A file is written only when it differs, and a file in reference/ the skill no longer ships
    # is removed; any other file in the folder is the project's and stays.
    set -euo pipefail
    from=.decree/machines/skill/scripts/skill
    dirs=()
    for dir in .claude/skills/decree .github/skills/decree; do
      [ -d "${dir}" ] && dirs+=("${dir}")
    done
    if [ ${#dirs[@]} -eq 0 ]; then
      cli=${AI_CLI:-}
      if [ -z "${cli}" ]; then
        for c in claude copilot; do
          if command -v "${c}" >/dev/null 2>&1; then cli=${c}; break; fi
        done
      fi
      case ${cli} in
        claude) dirs=(.claude/skills/decree) ;;
        copilot) dirs=(.github/skills/decree) ;;
        *) echo "skill: no skill folder, and AI_CLI names no CLI that reads skills (claude, copilot)"; exit 0 ;;
      esac
    fi
    for dir in "${dirs[@]}"; do
      changed=0
      while IFS= read -r file; do
        rel=${file#"${from}"/}
        if ! cmp -s "${file}" "${dir}/${rel}"; then
          mkdir -p "$(dirname "${dir}/${rel}")"
          cp "${file}" "${dir}/${rel}.tmp" && mv "${dir}/${rel}.tmp" "${dir}/${rel}"
          echo "skill: wrote ${dir}/${rel}"
          changed=1
        fi
      done < <(find "${from}" -type f | sort)
      if [ -d "${dir}/reference" ]; then
        for file in "${dir}"/reference/*; do
          [ -e "${file}" ] || continue
          if [ ! -e "${from}/reference/$(basename "${file}")" ]; then
            rm -f "${file}"
            echo "skill: removed ${file}"
            changed=1
          fi
        done
      fi
      [ "${changed}" = 1 ] || echo "skill: ${dir} is current"
    done
  skill/SKILL.md: |
    ---
    name: decree
    description: >
      Work in a decree project: messages (inbox, cron, order files), machines (YAML statecharts in
      .decree/machines/<name>/machine.yml) and their scripts, checked with `decree check`, drawn with
      `decree graph`, and queued with `decree emit`. Prefers the simplest machine: states are added
      only when the user asks or a run shows the need.
      INVOKE when: the user mentions decree or .decree/; writes or edits an order file, inbox message,
      cron file, machine or script; asks how to automate, schedule or chain work, ask a model or a
      person to decide a step, or why a run failed, is waiting or was interrupted.
      SKIP for: shell scripting, CI or infrastructure work unrelated to decree.
    ---

    # decree

    decree runs work through three building blocks:

    - A **message** (markdown) says *what* to do in its body, and *which machine* does it
      (`machine:` in the frontmatter). Migrations, inbox messages and cron templates are all messages.
    - A **machine** (`.decree/machines/<name>/machine.yml`) says *in what order*: a W3C SCXML statechart
      written in YAML. States, what each state invokes, and which event leads to which state. No
      code, no paths.
    - A **script** (`.decree/machines/<machine>/scripts/<name>.sh`, in the folder of the machine
      that runs it) does one piece of work and reports one outcome: exit 0 is `done`, non-zero is
      `error`, or the script names a richer event in a file (`echo pass > "$DECREE_EVENT_FILE"`).

    Every state invokes one function, and its result is an event. The function is a script, a child
    machine (`machine: deploy`), or a built-in decision: `check` (deterministic: `true` or `false`),
    `model` (a model picks an option, asked through a router machine) or `reply` (the run pauses
    until a reply, from a person or a program, picks one). `invoke` has exactly one key, which names
    the kind. AI and people appear only where a machine invokes `model` or `reply`.

    ## Starting a project

    ```bash
    decree init                  # .decree/ with the plugins ai, order, prune and skill
    decree list                  # this project's plugins, then the official ones it can add
    decree install develop       # develop, and infra, which it needs, into .decree/plugins/, installed
    ```

    `init` never prompts and installs no plugin. Add only what the project needs; each plugin's
    file (`.decree/plugins/<name>.md`) says what it does, what it needs and how to use it.
    For a Rust, Node or React project, add the `develop` variant that fits instead of `develop`:
    `decree install develop_rust` (or `develop_node`, `develop_react`). Each is `develop`'s
    machine under its own name, with a gate that already runs the ecosystem's standard checks,
    conventions its prompts follow, and its tools in `machines/<variant>/scripts/requirements`; a project
    with two parts (a Rust API, a React front end) adds two and names one in each message. A
    project in another ecosystem starts from `develop` and fills in its gate.

    ## Rules

    - **Start with the simplest machine.** Write the most naive machine that does the job: the
      scripts it needs, one state each, in a straight line (`done` → the next state), ending in
      `done`. `error` already goes to `failed` implicitly; don't write it.
      - Add nothing else until the user asks for it or a run has shown it is needed: no `check`,
        `model` or `reply` decisions, loops or round counters, `data` params, `attempts`,
        `timeout`, child machines, `emits`, compound states or one state per case.
      - When an addition is warranted, add the smallest one that solves the problem, and say why
        in the state's comment (`# the local model fails often: try Claude second`).
      - The cheapest fixes come first, in this order: change the script; `attempts`; a transition
        on an event the script names; a `check`; a `model` or `reply` decision; a child machine.
      - Asked to design a machine, show the simple version, and list possible additions in prose
        as options, not in the YAML.
      - When `develop` runs show that a failed verify is worth more rounds, decree's
        [escalation ladder](https://github.com/jtmckay/decree/blob/main/docs/escalation.md)
        recipe adds them: rounds with a cheap agent, one with Claude, then a person.
    - **Ordered work is an order file**, `.decree/machines/<machine>/order/*.md`: the official
      `order` plugin runs them at each daemon start (`decree daemon`, `decree daemon --once`), one at
      a time in byte order of filename, each only once `.decree/inbox/` is empty, and records each in
      that machine's `state/order.git.txt` with its hash. An order file says how things should be:
      editing it runs it again, so write it to be safe to re-run. A failed one stops the files after
      it until it passes.
    - **Name order files so they keep their order.** `100-` sorts before `99-`: pad the number to
      the width the project will need (`001-`), and when a project outgrows it, continue with a
      letter prefix (`A01-`, then `B01-`), because letters sort after digits.
    - **One concern per message**, day-sized, with Given / When / Then acceptance criteria whose
      outcomes are observable (exit codes, file contents, output).
    - **Describe the outcome, not the steps.** A message says what is true when it is done and
      why; the agent chooses how, as `infra` declares a desired state and `apply` reaches it.
      - **Outcome** (belongs in a message): behaviour a user or another program can observe; the
        public contract (command and flag names, file names, formats and locations, exit codes,
        messages printed); what must not change; why the change is wanted.
      - **Steps** (left out): which functions, modules or files to change, in what order, which
        data structures. Name one only as a constraint ("no new dependencies", "decree itself runs
        no git command"), never as a suggestion.
      - The test for any sentence: if a different correct implementation would fail it, it is a
        step.
      - The body: `## Overview` (why), `## Outcome` (what is true afterwards), `## Constraints`
        (what must hold or not change), `## Acceptance Criteria` (Given / When / Then,
        observable). `reference/messages.md` shows a step rewritten as an outcome.
    - **Always set `machine:`** in a message. Pick from `.decree/machines/` (`ls .decree/machines`).
    - **Before writing a machine, read
      `.decree/schema/v1/machine.schema.json`** (`decree install` writes it), the JSON Schema of every machine key, and write
      against it. Read `events.schema.json`, `request.schema.json` and `reply.schema.json` in the
      same folder before writing a router or anything that reads a run.
    - **Scripts choose how, machines choose what runs next.** A script may decide *how* to do its
      one job from its params and environment: which workflow file, which model for this attempt,
      which flags. A choice of *which step runs next* is a transition, on `done` or an event the
      script names. Make it a decision state (`check`, `model`, `reply`) only when the step after
      it differs, and the choice is worth seeing in the graph, testing with `decree check`, or
      giving to a model or a person.
    - **Route with a typed router.** For a `model` decision that routes work (which model, which
      path), point `router:` at a typed router (a classifier such as GLiNER2.5-Decide, or a model
      constrained by `reply_schema`): it cannot answer outside the options and costs little. Keep
      untyped models (`claude -p`) for the work itself. To combine several routers by a quorum, use
      the [consensus recipe](https://github.com/jtmckay/decree/blob/main/docs/routers.md#consensus-of-several-routers)
      (a router machine, no new decision kind).
    - **Queue follow-up work with `decree emit`**, never by writing into `.decree/inbox/` by hand
      from a script. The emitting state must list the target in `emits:`.
    - **A file a person writes into `.decree/inbox/` runs in the order it arrived**: the first
      `daemon` to see it gives it a fresh id, renames it `<id>.md` and records its old
      name as `arrived_as`. decree replaces `id`, `state` and `trigger` there, so to run a message
      again (an order file, a run's `message.md`), run `decree emit < <file>` or copy it into
      `inbox/`.
    - **Run `decree check` after every change** to a machine, script, message or cron file, and
      `decree graph` after changing a machine. Commit each `machines/<name>/graph.md` and
      `.decree/system.md`.
    - **Read decree's output as JSON, never by parsing its text**: `decree check --format json`,
      `decree status [<id>] --format json`, and `--format json` on `emit`, `reply`, `retry`,
      `cancel`, `pause`, `resume`, `graph`, `upgrade`, `install`, `list` and `export` print one
      JSON document, described by
      `.decree/schema/v1/cli/<command>.schema.json`. Exit codes are the same as in text.
      `decree check --format json` gives each error's `rule`, `file`, `line` or `state` and
      `message`.
    - **Scripts must be safe to re-run**, after an interruption or a failure halfway:
      `decree retry` runs the step again. Check that the result works (`pip --version`
      runs), not that a file exists.
    - **Keep a step's previous output until it has a new one.** A script that rewrites a file
      the next attempt or round reads (`gaps.md`, a report) sets the old one aside and puts it
      back if the attempt dies, so a crashed attempt never leaves the retry with nothing:

      ```bash
      [ ! -f "${out}" ] || mv "${out}" "${out}.last"
      trap '[ -s "${out}" ] || [ ! -f "${out}.last" ] || mv "${out}.last" "${out}"' EXIT
      ai "${prompt}"            # writes ${out}
      rm -f "${out}.last"
      ```

    - **Call AI only through `ai`** from `ai.sh`, never `claude` or `opencode` directly: it
      waits out Claude's usage limit, nudges OpenCode back after a rejected write, logs every call
      in `.decree/ai-use.md` and keeps the agent from naming an event. Pick the backend with a
      data entry `ai_cli` (a message's or a parent's `params` set it per run), which wins over
      the project's `AI_CLI`. Every prompt says where scratch files go
      (`$DECREE_RUN_DIR`): OpenCode's `external_directory` permission covers its file tools, not
      its shell, so it is no sandbox.
    - **Declare the environment.** A command, package or tool the project needs is a line in
      `.decree/machines/infra/scripts/requirements`, added by a message for the `infra` machine, never
      installed by another machine's step. Nothing runs `sudo`: a step only a person can do prints
      the exact command and fails, and the person runs it, then `decree retry`.
    - **Check a boundary with a script, not only a prompt.** When an agent must not change
      something (QA never edits app code), a script after it checks, and fails the step if it did.
    - **A machine's helpers, config and data go in `machines/<machine>/scripts/`**, beside the scripts its
      states invoke (`ai.sh`, a prompt template, a workflow file); decree runs only what a state
      names, and scripts reach the rest by path from the project root. Variables and secrets go in `.decree/.env` (every script) or a machine's
      `env_file: .env.<name>` (that machine's scripts only); both are gitignored, and
      `.decree/.env.example` is the committed template.
    - **What a machine keeps between runs goes in `$DECREE_STATE_DIR`** (`.decree/machines/<machine>/state/`)
      and is declared under the machine's `state_files:`, with what it is and which states read or write
      it. A run's own files go in `$DECREE_RUN_DIR`; helpers, config and data in `machines/<machine>/scripts/`;
      project-wide files no machine owns (a test tool's browsers) in `.decree/cache/`. Never
      `tee` decree's output: `daemon` keep it in `.decree/process.log`.
    - **A step that waits for a shared resource is an invoked state with a `timeout`** (ComfyUI's
      queue, a model unloading), not an `onentry` script, which has none. Keep `onentry` for quick
      steps.
    - **What needs a person is in `.decree/attention/`.** Read it first when a run is failed,
      waiting or stuck, and do what each file says: `decree reply <wait id> <option>` for a wait,
      `decree retry <id>` for a failed or interrupted run. Never edit an attention file
      to answer; never write one from a machine's script. A step whose failure must stop every
      run writes `$DECREE_RUN_DIR/hold` (what `keep` does), and decree turns it into a holding
      attention file. Any `.md` file a person or another tool puts in the folder halts the queue
      until it is deleted. To pause the queue, `decree pause`: the runs going now finish their
      step, and nothing more runs until `decree resume`; Ctrl-C interrupts the runs instead.
    - **Plugins are the source.** Each `.decree/plugins/<name>.md` holds one machine folder,
      `.decree/machines/<name>/`: its `machine.yml` (or none: a library of scripts), `scripts/`,
      `cron/`, and seeds for its `state/` and `order/`; `x-` keys are for other tools. decree
      installs plugins only when asked: `decree install` (after a clone or a pull; `check`
      and `daemon` say when one is due), `add` or `import`. Edit the installed copy
      (`machine.yml`, `scripts/`, `cron/`), run it, then keep the edit with
      `decree export <name>`, or let the next run's `commit` export it, and commit the
      plugin file. A machine written by hand in `machines/<name>/` is not committed until it is exported
      (`decree check` warns about it). A project's settings for a plugin (`gate.sh`,
      `requirements`, `QA_RUN`) are such edits: they live in its plugin file. Export an edit
      before the next `decree install`: an install never stops over one, it archives the edited
      copy as a plugin file, `.decree/archive/<name>/<time>.md`, and installs the plugin over
      it (`decree check` warns while one is pending). `decree install` that file restores it.
      Never edit `.decree/cache/plugins/`.
    - **Never delete run folders to clean up**; they are the record. The official `prune` plugin
      deletes finished ones older than `PRUNE_OLDER_THAN` (30d) each week.
    - Commit `.decree/plugins/`, `.decree/system.md` and each `machines/<name>/graph.md` and
      `order/`; git ignores the rest of `.decree/machines/`, which `decree install` installs
      from `plugins/`, except any `*.git.*` file (`order.git.txt`) in a machine's `state/`. Do not commit
      `.decree/inbox/`, `.decree/runs/` or `.decree/cache/`.

    ## Commands

    | Command | Use |
    | --- | --- |
    | `decree check [--format json\|sarif]` | Validate every machine, script name, inbox message and cron file. Exit 1 lists one error per line (`json`: one document; `sarif`: a SARIF 2.1.0 log for code scanning). |
    | `decree install` | Install every plugin in `.decree/plugins/` as its file says: the step after a clone or a pull. Moves an installed copy edited in place to `.decree/archive/` first. |
    | `decree install <name>... [--official]` | Install these plugins, archiving an edited installed copy; an official plugin not in `.decree/plugins/` is copied there first, with those it needs. `--official` takes this decree's built-in copy, prints the edits it replaces and archives the old file. |
    | `decree install <file\|->`, or `curl -fsSL <url> \| decree install -` | Check a plugin file, copy it to `.decree/plugins/` and install it. |
    | `decree list` / `decree export [<name>]` | List the project's plugins (`official` or `own`, `differs`, `not installed`, `edited`), the machines no plugin installed, and the official plugins it can add. Write `.decree/plugins/<name>.md` from the installed `machines/<name>/` (its `machine.yml`, `scripts/` and `cron/`); with no name, every plugin whose installed copy was edited. |
    | `decree graph` | Write `.decree/machines/<machine>/graph.md` (Mermaid) for every machine, plus `.decree/system.md`. |
    | `decree install --official skill` | After upgrading decree: take its `skill` plugin; the next daemon start rewrites this skill, keeping other files in the skill folder. |
    | `decree` | Print a synopsis of the quick start and run nothing; run the queue with `decree daemon --once`. |
    | `decree upgrade [--yes] [--format json]` | Print how to upgrade decree (the install line, pinned with `\| sh -s -- <tag>`, and `cargo install --git … --force` for a cargo copy) and what to run in each project after, then ask y/N before running the install line, and then before running `decree install` in this project; `--yes` runs both without asking. After an upgrade, run `decree install` in each project: it writes the new schemas and installs the plugins. An agent never answers `y` for a person. |
    | `decree emit [--machine <m>] [--param k=v]...` | Queue a message from stdin: a body for machine `m`, or a whole message whose frontmatter the flags override (`decree emit < <file>` runs a message again). Prints the new id. |
    | `decree daemon --once [--quiet] [--machine <m> \| --exclude-machine <m>]` | Start (firing `@reboot` cron files, so `order` runs the order files), run everything queued: replies, pending runs, the inbox (FIFO), printing each run's output, then exit. `--quiet` for cron and CI. `--machine` runs only those machines, `--exclude-machine` all but those (repeatable, comma-separated; or `DECREE_MACHINES`, `DECREE_EXCLUDE_MACHINES`), so several processes can share one folder. |
    | `decree retry <id> [--state <s>]` | Make a failed or interrupted run pending again; a running `decree daemon`, or the next `decree daemon --once`, continues it. The failure message prints the exact command. |
    | `decree daemon [--interval <duration>] [--machine <m> \| --exclude-machine <m>]` | The same, in a loop, with scheduled cron, every `2s` by default; `--machine` and `--exclude-machine` as for `--once`. |
    | `decree status [<id>] [--cron] [--all] [--format json]` | Starts with `Attention:` (the files in `.decree/attention/`, those that hold first), then runs by status (`--all`: every run), and active runs in projects below; one run's events; `--cron`: cron files and when each fires next (`daemon start` for `@reboot`). |
    | `decree tail [<id>]` | Follow each run's output as it comes, or one run's until it stops. |
    | `decree reply <wait id> <event> [-m <note>]` | Answer a run waiting in a `reply` state; a run id means its current wait. |
    | `decree cancel <id> [-m <note>]` | End a waiting, pending or interrupted run, and the child runs below it, in `failed`; no script runs, and no attention file stays. `decree retry` continues it. |
    | `decree pause [-m <note>]` | Let the runs going now finish their step, then run nothing more until `decree resume` (no restart needed). Ctrl-C interrupts the runs instead. |
    | `decree resume` | Go on after `decree pause`. |

    `daemon --once` exits once nothing more can run, leaving `reply` waits for the next start;
    `daemon` keeps polling, fires scheduled cron and continues runs as replies arrive, so unattended
    work runs under `daemon`. Both read machines and `.env` files again before each run: an edit
    applies to the next run without a restart, and an edit that fails `decree check` is not used
    until fixed (the old machines keep running and the errors are printed).

    ## The official plugins

    A project may have added official plugins (`decree list` marks them `official`): `develop`
    (or its variants `develop_rust`, `develop_node` and `develop_react`, which work as it does),
    `infra` and `qa` below, `router` (the default router), and `newsletter`, `comfy`,
    `develop_by_size`, `gliner_router`, `llm_router`, `lease` and `plugin_install` (installs the
    plugins each time the daemon starts), each described in its plugin file. When one
    of the three below is there, start from it before writing a machine of the same kind. Each starts with `clean_tree` (a root `onentry`: it fails the run
    unless the git work tree is clean, naming each `interrupted` or `failed` run that may have left
    the changes, with its `decree retry` and `git stash push` commands, also in its attention file) and
    ends with `commit` (`done`'s `onentry`: the run's work as one commit, titled with the message's
    first heading, and the order file for an order file's run, recorded in the same commit), or, in `failed`, with `keep` (the
    run's changes stay in the work tree, and `keep` writes the run's `hold`, so its attention file
    in `.decree/attention/` holds every run of their `worktree` group until the run is retried; other machines go on). All three touch only the
    project folder. Before queuing work for them, commit or stash any changes. To continue: answer
    any `STOP` in `plan.md`, delete `STOP`, `decree retry <id>`, which builds on the
    kept changes and removes the hold. To drop the run: stash its changes and delete its
    attention file. Never run `develop`, `infra` or `qa` in two processes in one checkout (they
    share the work tree through `clean_tree` and `commit`); give each its own git worktree. For divergent work,
    or to keep editing while runs go on, suggest a
    git worktree the user makes and runs decree in (`git worktree add ../app-exp -b exp`, then
    `decree daemon --once` there): each checkout has its own runs and inbox, and its branch its own
    `order.git.txt` records. A new worktree has committed files only: copy `.decree/.env` in and run
    `.decree/machines/infra/scripts/run.sh apply` there first.

    `develop`: `precheck` →
    `implement` → `gate` → `verify`, with `fix` and `final_gate` only when the gate fails. Its scripts
    source the `ai` library's `ai.sh` (`ai`, which calls `ai_<backend>`: a data entry `ai_cli`, else
    `AI_CLI`, else the first of `claude`, `opencode` and `copilot` on `PATH`) and
    follow the run-directory files in `reference/scripts.md` (`progress.md`, `STOP`, `gate.log`,
    `plan.md`): a person answers a `STOP` in `plan.md`, deletes `STOP` and
    retries. `precheck` runs
    `machines/infra/scripts/run.sh check`, which checks every `.decree/machines/*/scripts/requirements`. The project fills in `machines/develop/scripts/gate.sh` with its own checks, kept by
    `decree export develop` (or the next `commit`); until then it runs nothing, says so and
    names `pass`. The gate names `pass` (to `verify`) or `fail` (to `fix`,
    then `final_gate`, whose `fail` ends the run); a non-zero exit means the checks could not run,
    such as a missing tool, and goes to `failed` without `fix`. `ai.sh` runs the agent with
    `DECREE_EVENT_FILE=/dev/null` and empties the caller's event file after it, so a gate the agent
    runs names no event for its caller. `verify`
    ends in `pass` or `fail` from the AI's `VERDICT:` line.

    `infra`: `declare` → `apply`. The AI edits `machines/infra/scripts/requirements` (`kind name version
    source`, one resource per line, in dependency order), adding a kind in `machines/infra/scripts/kinds/`
    only when none fits; `apply` runs `machines/infra/scripts/run.sh apply`, which installs what is missing and checks
    every line. The first `infra` order file builds the environment; later ones add to it. Its
    `commit` exports `infra`, so the new lines are in `.decree/plugins/infra.md` in the run's commit.

    `qa`: `recon` → `attack` → `run` → `triage` → `report`. The AI writes chaos tests in
    `QA_DIR` (`tests/chaos`) against `machines/qa/scripts/invariants.md`, each naming a wrench from
    `machines/qa/scripts/wrenches.md`; triage marks each real bug's tests with the framework's strict
    expected-failure marker and writes one report per root cause (the broken invariant and the
    expected behaviour, no fix), and `report` emits each as a `develop` message. A guard (`machines/qa/scripts/qa.sh`) fails any step whose AI changed a file outside
    `QA_DIR`. It does not start until `QA_RUN` in `machines/qa/scripts/qa.sh` names the suite's command (kept
    by `decree export qa`); once
    qa finds bugs, the gate should run it too.

    ## Worked example

    An order file for the `feature` machine, `.decree/machines/feature/order/01-rate-limit-upload.md`:

    ```markdown
    ---
    machine: feature
    ---
    # Rate-limit /api/upload

    ## Overview

    One API key can flood uploads and slow the service for every other key.

    ## Outcome

    Each API key makes at most 10 uploads per minute on `POST /api/upload`.

    ## Constraints

    - Other endpoints respond as before.

    ## Acceptance Criteria

    - **Given** a key that has uploaded 10 times in the last minute
      **When** it uploads again
      **Then** the response is 429 with `Retry-After`
    ```

    The smallest machine:

    ```yaml
    # Graph: graph.md
    name: hello
    description: Run one script.
    initial: greet
    states:
      greet:
        invoke: greet                  # runs scripts/greet (or machines/hello/scripts/greet)
        transitions: { done: done }    # exit 0 -> done; non-zero -> failed, implicitly
      done:   { final: true }
      failed: { final: true }          # every machine has one
    ```

    A machine grows one reason at a time. First, the straight line that does the job:

    ```yaml
    # Graph: graph.md
    name: develop
    description: Implement a message, then test it.
    initial: implement
    states:
      implement:                       # the job needs two scripts: one state each, in a line
        invoke: implement
        transitions: { done: test }
      test:
        invoke: test
        transitions: { done: done }
      done:   { final: true }
      failed: { final: true }
    ```

    Real runs then fail at random in `implement`, and a re-run passes: the cheapest fix is
    `attempts: 3` on its invoke. Later the local model often fails where Claude would not: give
    each attempt a value instead of adding a state, and say why.

    ```yaml
      implement:
        invoke:                        # the local model fails often: try Claude third
          script: { name: implement, attempts: [local, local, claude] }
        transitions: { done: test }
    ```

    The script picks the model from the attempt, a choice of *how*, not of what runs next.
    `attempts` retries after `error` only; to have a stronger model answer a weaker one's `STOP`,
    transition to a child machine.

    ```sh
    case "$DECREE_ATTEMPT_VALUE" in
      claude) claude -p "$(cat "$DECREE_MESSAGE")" ;;
      *)      ollama run qwen3:8b "$(cat "$DECREE_MESSAGE")" ;;
    esac
    ```

    ## Reference files

    Read the one you need; don't load them all upfront:

    - **`reference/messages.md`**: frontmatter keys, order files, the inbox,
      cron files, `decree emit`, replies to a waiting run.
    - **`reference/machines.md`**: the graph line, every machine key, the invoke types, conditions, choices and
      routers, composition, events and transitions, validation rules, full examples.
    - **`reference/scripts.md`**: where scripts live, how they run, how they report an event,
      every `DECREE_*` variable.
    - **`reference/runs.md`**: run folders, `events.jsonl`, run status, waiting and interrupted
      runs, `decree retry`, graphs.

    The reference files sit next to this file, in `reference/`.
  skill/reference/machines.md: |
    # Machines

    A machine is a statechart in `.decree/machines/<name>/machine.yml`: a W3C SCXML document written in
    YAML. Keys are SCXML's names (`initial`, `states`, `transitions`, `onentry`, `onexit`, `invoke`,
    `data`, `final`); decree implements a strict subset of SCXML, plus a few marked extensions. If
    you know SCXML, you know how a machine behaves.

    The first line links the machine to its graph:

    ```yaml
    # Graph: graph.md
    ```

    **Before writing a machine, read `.decree/schema/v1/machine.schema.json`**
    (written by `decree install`, ignored by git): the precise contract for every key below, with a description and
    examples for each. Write against it, then run `decree check`, which also checks what a schema
    cannot: that names resolve, targets exist and every state is reachable. Editors apply the same
    schema by path; machines carry no schema line.

    ## Keys

    | Where | Key | Meaning |
    | --- | --- | --- |
    | Root | `name` | Equals the file stem; `^[a-z][a-z0-9_]*$`. |
    | Root | `description` | Required. Shown in prompts, `decree status` and graphs. |
    | Root | `data` | `name: { type: string\|int\|number\|bool, default: ..., enum?: [...] }`. `number` is an int or a float; `enum` (string only) lists the allowed values, which the default and `params` must be one of (V14). Read-only; a message's `params` override defaults. Scripts see `DECREE_DATA_<NAME>`. |
    | Root | `state_files` | `name: description`: what the machine keeps between runs in `.decree/machines/<machine>/state/` (`$DECREE_STATE_DIR`). A name is a file or folder directly in that folder, `^[A-Za-z0-9][A-Za-z0-9._-]*$`; a description says what it is and which states read or write it (V14). `decree check` warns about undeclared files there. |
    | Root | `concurrency` | How many runs of this machine may execute at once, across processes (default 1). Machines run beside each other; a waiting run holds no slot (V14). |
    | Root | `concurrency_group` | Machines naming one group share its slots, such as those editing one work tree (`worktree`: `develop`, `qa`, `infra`); each gives the same `concurrency` (V14); no chain of invokes may lead from a group back to it (V20). |
    | Root | `env_file` | `.env.<name>`: a gitignored dotenv file in `.decree/` whose variables this machine's scripts get, over `.decree/.env` and the process environment; no other machine's scripts see them (V14). |
    | Root | `onentry`, `onexit` | Scripts run once when the run starts, and once after a root final state is entered. |
    | Root | `initial`, `states` | Required. `initial` is a direct child. |
    | State | `invoke` | The state's function (below). `attempts` and `timeout` go inside it. |
    | State | `transitions` | `event: target`, or `event: { target, description, type: internal }`. |
    | State | `onentry`, `onexit` | Scripts run every time the state is entered or exited. They produce no event. |
    | State | `initial`, `states` | Make the state compound (no `invoke`). |
    | State | `final: true` | A final state: only `description`, `onentry` and `emits` allowed. |
    | State | `description` | Optional; the question context for a model. |
    | State | `emits` | Extension. Machines this state's scripts may `decree emit` messages for. |

    Inside a state, write keys in this order: `description`, `invoke`, `onentry`, `onexit`,
    `transitions`, `emits`, then `initial` and `states` for a compound state. Don't write defaults
    (no `attempts: 1`).

    Every machine has a root-level final state named `failed`.

    ## Invoke: the state's function

    `invoke` is a map with exactly one key, which names the kind. For `script` and `machine`, a bare
    name is short for `{ name: <name> }`, and `invoke: implement` is short for
    `invoke: { script: implement }`.

    | `invoke` | What runs | Events |
    | --- | --- | --- |
    | `script: { name: <script>, attempts?: <n> \| [<value>, …], timeout?: <duration> }` | The script (see `scripts.md`), re-run in place while it fails, once per attempt (default 1), each stopped after `timeout`. A list gives each attempt its value in `$DECREE_ATTEMPT_VALUE`. | `done` (exit 0), `error` (non-zero), or the event it writes to `$DECREE_EVENT_FILE`. |
    | `check: <condition>` | decree evaluates the condition. No AI. | `true` or `false`. |
    | `model: { question: ..., router?: <machine>, min_confidence?: 0.8, output?: <state> }` | A router machine asks a model to pick one of the state's transitions. | An option; `unsure` below `min_confidence`; `error` if the router fails. |
    | `reply: { question: ..., notify: <script>, timeout?: <duration> \| timeout_at?: <file>[#<pointer>], attention?: false }` | The `notify` script tells someone (a person, a UI, a program); the run pauses for a reply. `timeout_at` ends the wait at an instant a script wrote in the run's folder (`lease.deadline`, or `job.json#/due`). | An option; `error` on timeout. |
    | `machine: { name: <machine>, params?: {...} }` | The machine runs as a child run. | The child's root final state (`failed` becomes `error`). |

    A duration (`timeout`, `decree daemon --interval` and the `prune` plugin's `PRUNE_OLDER_THAN`) is a whole
    number followed by one unit, `s`, `m`, `h` or `d`: `90s`, `10m`, `12h`, `7d`. No fractions, no
    combinations (`1h30m`) and no bare numbers.

    A state with no `invoke` and a `done` transition passes straight through. Write decision invokes
    in block style:

    ```yaml
      rounds_left:
        invoke:
          check: { visits: implement, less_than: { data: max_rounds } }
        transitions: { true: triage, false: review }   # true and false need no quotes
    ```

    ### Choices

    For `model` and `reply`, `question` is what is being decided, and the **options are the state's
    transitions** except `unsure` and `error`. Every option needs a `description`: the question and
    the descriptions are exactly what the model, or whoever replies, sees, so write them to stand alone. Write
    prose (`question`, `description`) in block style, one key per line. Prose containing `: ` breaks
    a plain YAML value: quote it (`description: "Step one: build"`) or use `description: >-` with the
    text indented below.

    A model sees only what it is given: the latest script output of the state named by `output`, and
    the message body. Without `output`, it gets no script output at all. A script decides what a model
    sees by what it prints, which is also how to keep secrets out of a prompt.

    ### Routers

    A `model` state never calls a model itself. decree writes `request.json` (question,
    options, the `output` state's output as `input`, message body, the run's history, and
    `reply_schema`, a JSON Schema for the reply; `.decree/schema/v1/request.schema.json`) and runs a
    **router**: an ordinary machine (`router:` on the invoke, else the machine named `router`, which
    the official `router` plugin provides).
    Its script reads `$DECREE_REQUEST`, asks a model, and writes
    `{"event": "...", "reason": "...", "confidence": 0.86}` to `$DECREE_REPLY`
    (`.decree/schema/v1/reply.schema.json`). decree checks the
    event is an option and applies `min_confidence`. To use another model, write another router
    machine and point `router:` at it, or replace `machines/router/machine.yml`; recalibrate `min_confidence` when you do.

    Routers are **typed** or **untyped**. A typed router cannot answer outside the options: a
    classifier such as GLiNER2.5-Decide picks one of the labels it is given, and a constrained
    decoder (Ollama's `format`) is handed `reply_schema` unchanged. An untyped router asks a chat
    model or coding agent (`claude -p`, Copilot, OpenCode) for free text, and its script must find
    and check the JSON; its confidence is self-reported. Use a typed router for routing decisions
    (cheap, frequent, bounded: which model, which path), and an untyped model for the work itself
    and for judgments that need reasoning over a lot of context. [docs/routers.md](https://github.com/jtmckay/decree/blob/main/docs/routers.md)
    shows both.

    ### Conditions

    Typed objects: exactly one subject, exactly one operator.

    | Subject | Value | Operators |
    | --- | --- | --- |
    | `output: <state>` | That state's latest script output, as logged. The state must invoke a script. | `matches` |
    | `data: <name>` | A `data` value. | `<op>`; `matches` for string data |
    | `visits: <state>` | Times that atomic state was entered in this run, since its last retry. | `<op>` |
    | `confidence: <state>` | Confidence of that `model` state's latest decision, 0 to 1. | `<op>` |

    `<op>`: `equals`, `not_equals`, `less_than`, `at_most`, `more_than`, `at_least`. `<value>` is a
    literal or `{ data: <name> }`; `int` and `number` data compare numerically. For example `{ output: read_text, matches: '(?i)invoice' }`. No
    `and`/`or`: use two `check` states in a row.

    ## Events and transitions

    - A script's event: `done`, `error`, or a name it writes to `$DECREE_EVENT_FILE`. Writing `done`
      is the same as writing nothing. Any other named event that is reserved or matches no transition
      becomes `error`.
    - Selecting a transition: the current state's `transitions`, then each ancestor's, innermost
      first. An `error` nobody handles goes to `failed`; any other unhandled event becomes `error`.
    - Event names match `^[a-z][a-z0-9_]*(\.[a-z0-9_]+)*$`. Scripts and options may not use
      `error`, `unsure`, or names starting with `done.` or `error.`, and options may not be `done`. A `check` produces `true` or
      `false`; YAML reads `true:` and `false:` as booleans, and decree reads them as those events.
      `yes` and `no` are ordinary event names (decree reads YAML 1.2, so they stay strings).
    - A **compound state** loops among its children until it enters its own final child, which
      raises `done.state.<id>`; handle it on the compound (`transitions: { done.state.work: done }`).
    - `onexit` runs from the innermost state outward, `onentry` from the outermost inward, as SCXML
      does. A self-transition exits and re-enters.

    ## Two kinds of retry

    | | `attempts` | A transition back (`retry`) |
    | --- | --- | --- |
    | Means | "That crashed; run it again." | "That worked but the result is wrong; do another round." |
    | Leaves the state? | No: no `onexit`/`onentry`, no new visit | Yes: `visits` + 1 |
    | Bounded by | `attempts` in the script invoke | A `check` on `visits` |

    `attempts` is a number, or a list whose entries run in order, each as `$DECREE_ATTEMPT_VALUE`,
    until one attempt does not end in `error`:

    ```yaml
    implement:
      invoke:
        script: { name: implement, attempts: [local, local, claude, local], timeout: 3h }
      transitions: { done: verify, error: ask_person }   # all four failed: a person
    ```

    Entries match `^[A-Za-z0-9][A-Za-z0-9._:/@-]{0,127}$` (`claude-opus-5-5`, `qwen3:8b`). Use a list
    when the same step is only done by something else (a stronger model); use a transition to another
    state when what happens next differs. Each new visit starts again at attempt 1.

    Only `error` (a non-zero exit or a timeout) runs the next attempt. `done`, or any event the
    script names, such as `stop`, ends the list at once and goes to the state's `transitions`.

    A stronger model answering a weaker one's `STOP` is not an attempt: transition to a child
    machine.

    ```yaml
    implement:
      invoke:
        script: { name: implement, attempts: [local, local] }
      transitions: { done: gate, stop: escalate }
    escalate:                        # a stronger model answers the local model's question
      invoke:
        machine: { name: escalate }
      transitions: { answered: implement }
    ```

    The child reads `$DECREE_PARENT_RUN_DIR/STOP`, appends the answer to
    `$DECREE_PARENT_RUN_DIR/plan.md` for the next `implement` visit, deletes `STOP` and ends in
    `answered`; a child that ends in `failed` reaches the parent as `error`, which goes to `failed`
    implicitly. Every caller must handle a child's final states, so keep them few.

    ## Not supported (and the alternative)

    `cond` on transitions (make the decision a `check` state), `<parallel>`, `<history>`, `<send>`
    (use `decree emit`), `<raise>` (a script names its event), `<assign>` (`data` is read-only),
    targetless transitions, XML. `decree check` names the alternative when it rejects one, and names
    the new shape for an old one (`choose`, `input`, a bare `matches`, the old retry key (now `attempts`),
    `timeout_s` on a state, `timeout_s` in an invoke, `{ machine: x, params: ... }`).

    ## Example: everything at once

    ```yaml
    # Graph: graph.md
    name: feature
    description: Implement one feature spec with an AI agent, verify it, and commit.
    data:
      max_rounds: { type: int, default: 2 }
    onentry: [clean_tree]              # root onentry: when the run starts, and again on a retry
    onexit: [notify]                   # root onexit: once, after a root final state is entered
    initial: precheck
    states:
      precheck:
        invoke: precheck
        transitions: { done: work }
      work:                            # compound: the implement/verify loop as one unit
        transitions: { done.state.work: done }   # raised when work reaches its own final state
        initial: implement
        states:
          implement:
            invoke:                    # attempts: re-run a crashing script in place
              script: { name: implement, attempts: 2 }
            onexit: [collect_logs]
            transitions: { done: verify }
          verify:                      # script: names pass or fail
            invoke: verify
            transitions: { pass: verified, fail: rounds_left }
          rounds_left:                 # deterministic check: true or false
            invoke:
              check: { visits: implement, less_than: { data: max_rounds } }
            transitions: { true: triage, false: review }
          triage:                      # a model picks one option; below the floor it is unsure
            invoke:
              model:
                question: Should we implement again or split the work?
                min_confidence: 0.8
                output: verify
            transitions:
              retry:  { target: implement, description: The failures look fixable; implement again. }
              split:  { target: spawn_followups, description: The scope is too large; emit smaller follow-up messages. }
              unsure: review
          review:                      # a person picks one option; the run pauses for the reply
            invoke:
              reply:
                question: Tests still fail. What next?
                notify: notify_person
                timeout: 2d
            transitions:
              approve: { target: verified, description: Good enough; commit it. }
              retry:   { target: implement, description: Try again; see my note. }
              reject:  { target: failed, description: Stop. }
          verified: { final: true }    # raises done.state.work
      spawn_followups:
        invoke: spawn
        transitions: { done: done }
        emits: [feature]               # extension: machines its scripts may emit messages for
      done:   { final: true, onentry: [commit] }   # an order file's run: commit records it in order.git.txt first
      failed: { final: true }
    ```

    ## Example: composing machines

    ```yaml
    # Graph: graph.md
    name: ship
    description: Implement a feature, then deploy it.
    initial: build
    states:
      build:
        invoke: { machine: feature }
        transitions: { done: release }
      release:
        invoke: { machine: deploy }
        transitions: { done: done, rejected: done }
      done:   { final: true }
      failed: { final: true }
    ```

    ## Escalation

    Decide the cheapest way first and pass on only what is undecided: `false` from a `check`, `unsure`
    from a `model` below its `min_confidence`, then a `confidence` check to decide whether a
    person's time is worth it, then a `reply` from a person. Each threshold is a number in the machine.

    ## Validation

    `decree check` runs rules V1–V21 (names, targets, `failed` exists, reachability, every script
    name resolves, decision states cover their events, options have descriptions, `emits` and
    `router` name real machines, no invoke cycles, nothing outside the SCXML subset) and M2–M3 (every
    inbox message and cron file parses and names a machine with valid `params`)
    and E1 (`.decree/.env` and each `env_file` are valid dotenv files without reserved keys). Errors look like `machines/feature/machine.yml: work.verify: <message>`. It also warns when
    a machine's `graph.md` or `.decree/system.md` is out of date, or `.decree/schema/` when it exists: run `decree graph` or
    `decree install`.
  skill/reference/messages.md: |
    # Messages

    A message is one markdown file: YAML frontmatter (structured) plus a body (the task, free text).
    One message is one run. decree reads and writes only the frontmatter and never changes the body.

    ```markdown
    ---
    machine: feature
    params:
      max_rounds: 3
    ---
    # Add rate limiting to /api/upload

    ## Overview
    ## Outcome
    ## Constraints
    ## Acceptance Criteria
    ```

    ## Frontmatter keys

    | Key | Who sets it | Meaning |
    | --- | --- | --- |
    | `machine` | Author, `decree emit`, cron | Machine name (`machines/<name>/machine.yml`). Required, but a cron file or order file runs its folder's machine without one. |
    | `params` | Author, `decree emit --param` | Values for the machine's `data` (string, int, number or bool), one of the `enum` values if the entry has one. Unknown names, wrong types and values outside the `enum` fail validation. |
    | `id` | decree only | `YYYYMMDDTHHMMSS.ffffffZ-xxxxxx`, UTC to the microsecond, so ids sort in the order messages arrived. Names the queued file and the run folder. |
    | `state` | decree only | Mirror of the run's current state, in `runs/<id>/message.md`. Never set it. |
    | `parent`, `depth` | `decree emit`, decree | The run that emitted this message (or invoked this child run), and the chain depth. |
    | `trigger` | decree only | `cron`, `emit`, `inbox` or `invoke` (a child run, as an order file's run is). |
    | `arrived_as` | decree only | The name a file written into `inbox/` had before it got its id. |
    | `traceparent`, `tracestate` | decree, `decree emit`, any tool | W3C Trace Context: the run joins this trace, under this span. An invalid `traceparent` is ignored and the run starts a new trace. |
    | `to`, `event` | `decree reply`, any tool | Only in a reply to a waiting run (below). |

    Any other key is kept as written and ignored.

    ## Order files: run in order at each start

    A machine's order files, `.decree/machines/<machine>/order/*.md` (committed), are the work that
    must happen in order: the official `order` plugin, which `decree init` adds, runs them each time
    the daemon starts (`decree daemon` and `decree daemon --once`).

    - **Name** them with a numeric prefix padded to the width the project will need:
      `001-add-login.md`, `002-rate-limit-upload.md`. They run in byte order of filename across every
      machine's `order/` (the machine's name breaks a tie), so `100-` sorts before `99-`. A project
      that outgrows the width continues with a letter prefix (`A01-`, then `B01-`), because letters
      sort after digits.
    - **The folder's machine.** An order file runs on its folder's machine unless it names another
      in `machine:`. It runs as a child run of the `order` run, with `order: <machine>/<file>` added.
    - **Once, and again when it changes.** Each file that finishes in any state but `failed` is
      recorded, `<file> <sha256>`, in its machine's `state/order.git.txt` (committed). A recorded file
      does not run again; an edited one does, so write each to be safe to re-run: declare how things
      should be, not a step to repeat.
    - **Strict order.** A file starts only once `inbox/` is empty, so the follow-ups the file before
      it emitted run first; a failed follow-up has left `inbox/`, so it holds nothing back. A file
      whose run fails stops the files after it; the next start runs it again.
    - **Committing.** The official `commit` script records the file in `order.git.txt` before it
      commits, so `done: { final: true, onentry: [commit] }` commits the work and the record together.
      decree never runs git itself.

    ### Writing a good message

    - One concern, day-sized: implementable in one agent session and reviewable on its own. Five
      subsystems means five messages.
    - Self-contained: no dependency on a sibling message's runtime state.
    - **Describe the outcome, not the steps.** A message says what is true when it is done and
      why; the agent chooses how. A message that says *how* ties the agent to the implementation
      its author pictured, and when the code differs, the run stops or the agent forces the plan.
      This is declarative configuration, as `infra` declares a desired state and `apply` reaches
      it.
      - **Outcome** (belongs in a message): behaviour a user or another program can observe; the
        public contract (command and flag names, file names, formats and locations, exit codes,
        messages printed); what must not change; why the change is wanted.
      - **Steps** (left out): which functions, modules or files to change, in what order, which
        data structures. Name one only as a constraint ("no new dependencies", "decree itself runs
        no git command"), never as a suggestion.
      - The test for any sentence: if a different correct implementation would fail it, it is a
        step.
    - The body has four sections:

      ```markdown
      # <title>

      ## Overview
      Why the change is wanted.

      ## Outcome
      What is true afterwards, as a user or another program sees it.

      ## Constraints
      What must hold or must not change.

      ## Acceptance Criteria
      Given / When / Then, each observable.
      ```

      A message with other headings is still valid and runs the same: decree never reads the
      body.
    - One sentence, written as a step and as an outcome:

      > **Step:** The warning has its own rule id, following the existing warnings.
      >
      > **Outcome:** `decree check` shows this warning and still exits 0, in text, JSON and
      > SARIF.

      The step assumed the existing warnings had rule ids; they had none, and the run stopped.
      The outcome holds for any correct implementation.
    - Acceptance criteria as Given / When / Then, each automatable:

      ```markdown
      - **Given** <precondition>
        **When** <action>
        **Then** <observable result: exit code, file contents, output, response>
      ```

      Avoid outcomes like "works correctly".

    ## Inbox: first in, first out

    `.decree/inbox/*.md` holds queued messages, run in the order they arrived. Not committed.

    - Each is named `<id>.md`. A file written under any other name (`01-hi.md`) gets a fresh id
      the first time `daemon` sees it, as `decree emit` would have given it, so it runs
      after everything queued before; files first seen together go by modification time, then
      name. Its old name is kept as `arrived_as`, in `runs/<id>/message.md` too.
    - `id`, `state`, `trigger` and `arrived_as` are decree's alone: a file placed in `inbox/` gets
      decree's own, so a copy of an order file or of a run's `message.md` runs again as a new run.
    - A failed inbox run does not stop `decree daemon --once`: it prints the run with
      `decree retry <id>` and goes on with the next file. When the inbox is empty it lists
      every run that failed in the pass and exits 1.
    - People may drop a file in directly. Programs write `.<name>.tmp`, then rename it to `<name>.md`;
      files starting with `.` are ignored, so nobody reads a partial file.
    - From a script, use `decree emit`, which does the temp-file-and-rename for you, sets `parent`,
      `depth` and `trigger: emit`, and refuses past `max_depth` (10 unless `DECREE_MAX_DEPTH` in the shell or `.decree/.env`
      sets it, a whole number of at least 1):

      ```bash
      "${DECREE}" emit --machine feature --param max_rounds=3 <<'EOF'
      # Follow-up: split rate limiting per route

      ## Overview
      ## Outcome
      ## Constraints
      ## Acceptance Criteria
      EOF
      ```

    - Inside a run, the emitting state must list the target machine in its `emits:`; otherwise
      `decree emit` exits 1. `decree graph` draws these edges in `system.md`.
    - Stdin that starts with frontmatter is a whole message: `decree emit` keeps its frontmatter
      (less the keys decree sets), `--machine` and `--param` override it, and `--machine` may be
      left out when it names a machine. Frontmatter that is not valid is refused, as
      `decree check` would, and nothing is queued.

    ### Run a message again

    ```bash
    decree emit < .decree/machines/develop/order/01-a.md
    ```

    or copy the file into `inbox/`. Either way it is a new inbox run with the same frontmatter and
    body; the order file and `order.git.txt` are untouched.

    ## Cron

    `.decree/machines/<machine>/cron/*.md` files are message templates that `decree daemon` drops into `inbox/` on a
    schedule. The `cron:` key is a standard cron expression; the rest is an ordinary message.

    ```markdown
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
    ```

    `cron: "@reboot"` (Vixie cron's word) queues the message once each time `decree daemon` starts,
    at its first pass, even while the queue is held; a daemon restarted after a crash queues it again,
    so the script checks its own result before redoing work. A `@reboot` file added while the daemon
    runs waits for the next start, and `decree daemon --once` never fires it.

    ```markdown
    ---
    cron: "@reboot"
    machine: boot
    ---
    Check that the services this project calls answer.
    ```

    `decree status --cron` lists them with their next fire time (`daemon start` for `@reboot`).

    ## Replies to a waiting run

    A run in a `reply` state waits for a reply with a **wait id** (`<run id>.w<seq>`). The
    state's `notify` script tells someone the question, the options and how to answer. A reply is an
    inbox message with `to:` (the wait id, or the run id meaning "its current wait") and `event:`
    (one of the options); the body is an optional note the run's later scripts read through
    `DECREE_RECEIVED`.

    ```bash
    decree reply 20261001T145120.000000Z-9c2e57.w15 retry -m "plans.toml lives in config/."
    ```

    writes:

    ```markdown
    ---
    id: 20261001T160301.482113Z-9be210
    to: 20261001T145120.000000Z-9c2e57.w15
    event: retry
    ---
    plans.toml lives in config/.
    ```

    Any tool (a chat bot, a web UI) can write the same file. A reply to a stale wait id or with an
    event that is not an option is not applied; it shows up in `decree status` as a failed message,
    with a file in `.decree/attention/` that never holds the queue (`holds: false`).

    With a `timeout`, a reply queued at or before the deadline wins over the timeout, even when
    `daemon` gets to it after the deadline. The time that counts is the reply's `id`,
    when it reached the queue: when `decree reply` or `decree emit` queued it, or, for a file
    written straight into `inbox/`, when decree gave it an id on arrival. A tool that answers near
    a deadline should queue its reply with `decree reply`; a reply queued after the deadline loses,
    and the run takes `error` with `timed_out: true`.
  skill/reference/runs.md: |
    # Runs

    One message moving through one machine is a run, stored in `.decree/runs/<id>/`:

    ```text
    runs/<id>/
      message.md                 the claimed message; decree mirrors `state:` into it
      events.jsonl               one JSON line per event: the record and the telemetry
      traces.jsonl               one OTLP/JSON span per line, for OpenTelemetry tools
      0001-<state>-<script>.log  output of each script execution, numbered in run order
      request.json, reply.json   in a router's child run
      received/                  replies delivered to this run
      .lock, .running            the process stepping the run, and the script running now
    ```

    ## events.jsonl

    The source of truth: the run's state is the `to` of its last `transition` event. Every line
    carries `v`, `seq`, `ts`, `type`, `run_id`, `machine`, `trigger` and `trace_id`; `script`,
    `decision` and `received` events, the claim and a `retry` carry the `span_id` of their span in
    `traces.jsonl`. Every field of every type,
    with its meaning, is in `.decree/schema/v1/events.schema.json`: read it.
    Types:

    | `type` | Meaning |
    | --- | --- |
    | `transition` | `from`, `event`, `to`, `source` (`claim`, `exit_code`, `script`, `attempt`, `check`, `model`, `reply`, `machine`, `timeout`, `internal`, `invalid_message`, `retry`). |
    | `script` | One script finished: `state`, `phase`, `script`, `path`, `attempt`, `duration_ms`, `exit_code`, `log`. |
    | `decision` | A `check`, `model` or `reply` produced its event: `kind`, `event`, and for models `pick`, `reason`, `confidence`, `router`, `child_run`. |
    | `waiting` | Paused for a person (`wait_id`, `options`, `timeout_at`) or a child run (`child`). |
    | `received` | A reply, a timeout or a child's result arrived. |
    | `interrupted` | Stopped before a final state: `cause` is `signal` or `crash`. |
    | `run_finished` | Reached a root final state: `state`, `duration_ms`. |

    To find out why a run failed, read its `events.jsonl` from the end, then the log the last
    `script` event names. `decree status <id>` shows the same as a table.

    ## Run status

    | Status | Meaning | What to do |
    | --- | --- | --- |
    | `finished` | Reached a final state (`done`, `failed`, ...). | Nothing; for `failed`, what its file in `.decree/attention/` says, `decree retry <id>`; or that to run it again from a state. |
    | `active` | A live process holds its lock. | `decree tail` to watch it. |
    | `waiting` | Paused for a person, or for a child run. | `decree reply <wait id> <event>`; the run's file in `.decree/attention/` and `decree daemon --once` give the commands. `decree cancel <id>` if nobody will answer. |
    | `pending` | A reply arrived, or a `retry` transition was written. | `decree daemon --once` continues it. |
    | `interrupted` | Stopped by a signal or crash. | Fix the cause, then `decree retry <id>`, as its file in `.decree/attention/` says, or `decree cancel <id>` to drop it. decree never continues it on its own. |

    A failed order file stops the order files after it: the `order` run fails, and the next start
    runs that file again. `decree daemon --once` exits 1 when a run failed in its pass, naming
    `decree retry <id>` for each.

    ## Attention

    `.decree/attention/<id>.md` is decree's list of what needs a person: a file per run that is
    `failed`, `interrupted` or `waiting` in a `reply` state. Frontmatter `kind: run`, `run`, `machine`, `message`,
    `status`, `state`, `since`, `holds` (`.decree/schema/v1/attention.schema.json`), then what
    happened, the exact command (`decree retry <id>`, or one `decree reply <wait id>
    <option>` per option), the run's `hold` text, its `leftovers` and its `STOP` question. decree
    writes and deletes the files as each run's status changes; a file a person deleted stays deleted
    until it changes again. `decree daemon --once` ends with `<n> run(s) need attention: .decree/attention/`, and
    `decree status` starts with `Attention:` (`attention` in `--format json`).

    - Read-only: answer with `decree reply` or a reply, continue with `decree retry`;
      editing a file answers nothing, and no script writes one.
    - A file with `holds: true` (its run wrote `hold`) holds the runs of its machine's
      `concurrency_group` (or of the machine alone): they neither start nor continue, and the
      other machines go on. `pause.md`, or a `.md` file there that cannot be read as one (no
      frontmatter, invalid frontmatter), holds every run. `daemon --once` exits 1 naming it. `decree retry <id>` removes the run's `hold`
      and its file; deleting the file drops the run (stash its changes first), and decree then
      deletes the `hold`. Other files and subfolders there are ignored.

    ## Graphs

    `decree graph` writes `.decree/machines/<machine>/graph.md` (a Mermaid `stateDiagram-v2`) for
    each machine and `.decree/system.md` (machines, `emits` and `invokes` edges, cron entry points).
    Edge labels say who decided: `(check)`, `(model)`, `(reply)`, `(machine: <name>)`; `(implicit)`
    marks an unhandled `error` going to `failed`. Commit the files; they render on GitHub, GitLab,
    Obsidian and in VS Code's Markdown preview (`Ctrl+Shift+V`), or paste the diagram into
    https://mermaid.live.

    Read a machine's graph before changing it, and run `decree graph` and `decree check` after.
  skill/reference/scripts.md: |
    # Scripts

    A script is any executable file; the shebang picks the language (bash by default). It does one
    piece of work, makes no routing decision, and reports one outcome.

    ## Where scripts live

    Machines name scripts, never paths. Script `X` used by machine `M` resolves in one folder only,
    `.decree/machines/M/scripts/`. There is no other search path: a script another machine ships is
    reached by path from a script, never by name. To share a machine and its scripts across projects, make it a plugin: `decree export M` writes `.decree/plugins/M.md`.

    A match is a file named `X` or `X.<ext>` (`verify.sh`), executable, and the only match in its
    directory. Script names match `^[a-z][a-z0-9_]*$`. Code several machines share is a library: a
    machine folder with no `machine.yml` (a plugin with no `machine`), whose scripts others source
    by path and name in their plugin's `needs`.
    `chmod +x` new scripts; `decree check` fails otherwise.

    A machine's helpers, config and data (bash functions to source, a prompt template, workflow
    files) sit beside its scripts in `machines/<machine>/scripts/`; decree runs only a file a state names,
    and other files there are never an error. Scripts reach them by path from the project root,
    their working directory: `. .decree/machines/develop/scripts/ai.sh`. What a machine keeps between runs
    goes in `$DECREE_STATE_DIR`.

    `machines/infra/scripts/` holds the project's declared environment. `requirements` lists one resource
    per line (`kind name version source`), in dependency order; any other machine may keep a
    `requirements` beside its scripts, and every `.decree/machines/*/scripts/requirements` is one list,
    infra's first; `kinds/<kind>.sh` defines `check` and
    `install` for a kind, each called with the line's name, version and source; `run.sh check`
    reports each line `ok` or `missing`, with the file it came from, and `run.sh apply` installs what is missing and checks it
    again. A kind installs into the project (a venv, `node_modules`, `.tools/bin`), never with
    `sudo`; a step only a person can do prints the exact command and returns 1. A kind that
    downloads checks the file against the sha256 in its line's source, `<url>#sha256=<hex>`, taken
    from what the project publishes (`SHA256SUMS`, the release page), never from hashing the
    download itself. The `infra` plugin ships
    the `command` kind (on `PATH`, at the line's version if it has one, from `--version`; or a
    person installs it).

    ## How a script runs

    - Directly (no shell wrapper), from the project root, with stdin `/dev/null`, in its own process
      group.
    - stdout and stderr go, interleaved and unmarked, to `runs/<id>/NNNN-<state>-<script>.log`
      (`NNNN-<state>.log` when the script is named as its state). What an invoke prints is what a later `matches` check or model reads. Output
      is only a log: decree never reads an event from it.
    - On stop or timeout, decree sends SIGTERM to the group, then SIGKILL after 10 s.
    - Scripts must be safe to re-run: an interrupted or failed step runs again after
      `decree retry`. Check that a result works, not that its file exists: a step that
      failed halfway may have left the file.

    ## Reporting an event (invoke only)

    Write the event's name to `$DECREE_EVENT_FILE`, a fresh empty file for each attempt
    (`echo pass > "$DECREE_EVENT_FILE"`; in Python,
    `open(os.environ["DECREE_EVENT_FILE"], "w").write("pass")`).

    1. Exit non-zero: `error`. The file is not read.
    2. Exit 0 and the file holds a name (whitespace trimmed): that event.
    3. Exit 0 and the file is empty, or holds `done`: `done`.
    4. Any other name that is reserved or matches no transition of the state or its ancestors: `error`.

    ```bash
    #!/usr/bin/env bash
    # machines/feature/scripts/verify.sh: a pass is deterministic; only failures go further.
    set -uo pipefail
    cargo test 2>&1 | tail -n 40
    if [ "${PIPESTATUS[0]}" -eq 0 ]; then
      echo pass > "$DECREE_EVENT_FILE"
    else
      echo fail > "$DECREE_EVENT_FILE"
    fi
    ```

    `onentry` and `onexit` scripts never produce events:

    - A failing `onentry` script skips the remaining `onentry` scripts of that state only (SCXML stops
      the failing block, `error.execution`). The other states being entered still run theirs,
      outermost first. Once entry is complete, the event is `error`, selected from the atomic state
      entered like any other event, and the invoke does not run. This holds for root `onentry` too:
      an `error` transition can handle it, and otherwise it goes to `failed`.
    - A failing `onentry` script on a root-level final state moves the run to `failed` instead of
      ending in that state, so a failed `commit` is never reported as `done`.
    - A failing `onexit` script is recorded in the `transition` event under `exit_failures`; the event
      and target do not change.

    `$DECREE_EVENTS` lists the events the current state accepts.

    ## Queueing follow-up work

    ```bash
    #!/usr/bin/env bash
    # machines/feature/scripts/spawn.sh: the state lists `emits: [feature]`.
    set -euo pipefail
    decree emit --machine feature <<'EOF'
    # Follow-up: split rate limiting per route

    ## Overview
    ## Outcome
    ## Constraints
    ## Acceptance Criteria
    EOF
    ```

    ## Run-directory files

    Scripts hand work to one another, and to a person, through files in `$DECREE_ROOT_RUN_DIR`
    (the run's own folder, or in a child run the top run's, so a child machine whose steps are
    develop's scripts shares them). decree reads none of them but `hold`, and quotes `STOP` and
    `leftovers` in the run's attention file; the official `develop` plugin's scripts follow this convention:

    - `progress.md`: the step log, one line per step, what is done and what is next. A retry
      reads it and continues.
    - `STOP`: a question instead of a guess. A script that finds it prints it and writes `stop` to
      `$DECREE_EVENT_FILE`; it keeps stopping retries until a person answers in `plan.md`, deletes
      it and runs `decree retry`.
    - `gate.log`: the output of the project's checks (`machines/develop/scripts/gate.sh`), for the step that
      fixes them. The gate names `pass` or `fail`; a non-zero exit (`error`) means the checks could
      not run, such as a missing tool, and its message is here.
    - `plan.md`: a plan for later steps, and a person's answer to a `STOP`. develop's AI steps read it.
    - `gaps.md`: what `verify` found missing on `fail`; a retry's `implement` closes each first.
      `verify` keeps the last one if an attempt dies.
    - `hold`, in the run's own `$DECREE_RUN_DIR`: written by a step whose failure must stop the
      runs that share its machine's group (the official plugins' `keep`, in `failed`). decree puts
      its text in the run's file in `.decree/attention/` with `holds: true`, and no run of that
      group runs until `decree retry <id>` removes both or a person deletes the attention file.
    - `leftovers`, in the run's own `$DECREE_RUN_DIR`: written by the official plugins' `clean_tree` when it
      finds uncommitted changes. It names each run that passed `clean_tree` and is now
      `interrupted` or `failed` (from `"${DECREE}" status --all --format json`, read with `jq`),
      with `decree retry <id>` and `git stash push --include-untracked -m "decree <id>"
      -- .`; `clean_tree` prints the same lines. decree quotes it in the run's attention file.

    ## Environment

    | Variable | Value |
    | --- | --- |
    | `DECREE` | The decree executable running the script; call it as `"${DECREE}"`, not `decree`, which may not be on `PATH`. |
    | `DECREE_PROJECT_ROOT` | Directory containing `.decree/`. |
    | `DECREE_MESSAGE` | `runs/<id>/message.md` (the run's copy of the message). |
    | `DECREE_MESSAGE_ID` | The message `id`. |
    | `DECREE_MACHINE`, `DECREE_STATE` | Machine and state (`_root` for root `onentry`/`onexit`). |
    | `DECREE_PHASE` | `onentry`, `invoke` or `onexit`. |
    | `DECREE_VISITS` | Times this state has been entered, including now, since the run's last retry. |
    | `DECREE_RUN_ATTEMPT` | The run's attempt: 1, then one more per `decree retry`. |
    | `DECREE_RUN_DIR` | `runs/<id>/`. |
    | `DECREE_STATE_DIR` | `.decree/machines/<machine>/state/`: what this machine keeps between runs; created before the script runs. |
    | `DECREE_ATTEMPT`, `DECREE_MAX_ATTEMPTS`, `DECREE_FINAL_ATTEMPT` | Attempt number, the limit, and `true` on the last one. |
    | `DECREE_ATTEMPT_VALUE`, `DECREE_ATTEMPT_VALUES` | With an `attempts` list: this attempt's entry, and the whole list, space-separated. Unset with `attempts: <n>`; no value for `onentry`/`onexit`. |
    | `DECREE_TRIGGER` | `cron`, `emit`, `inbox` or `invoke`. |
    | `DECREE_EVENTS` | Events the current state accepts, space-separated. |
    | `DECREE_EVENT_FILE` | For an invoke: the file to write its event to. Empty for other scripts. |
    | `DECREE_PARENT` | In a child run, the parent run's id. |
    | `DECREE_PARENT_RUN_DIR` | In a child run, the parent run's folder; empty otherwise. Reach the task's files from here, never by building `runs/$DECREE_PARENT`. |
    | `DECREE_ROOT_RUN_DIR` | The folder of the run at the top of the chain (no parent); `DECREE_RUN_DIR` in a top-level run. |
    | `DECREE_REQUEST`, `DECREE_REPLY` | In a router run: the request JSON to read and the reply JSON to write. |
    | `DECREE_WAIT_ID`, `DECREE_QUESTION`, `DECREE_CHOICES` | For a `reply` state's `notify` script: the wait id, the question, and a JSON file of options and descriptions. |
    | `DECREE_RECEIVED` | The last reply this run received (`runs/<id>/received/<file>`), or empty. |
    | `DECREE_DATA_<NAME>` | Each `data` value: the message's `params`, else the default. |
    | `TRACEPARENT`, `TRACESTATE` | W3C Trace Context: the run's trace and this script's span (`00-<trace id>-<span id>-01`), and the message's `tracestate` if it had one. OpenTelemetry SDKs read them, so the script's own spans join the run's trace; `decree emit` copies them into the new message. |

    Variables and secrets (an API key, a service URL, a model name) go in dotenv files in
    `.decree/`, which `.decree/.gitignore` keeps out of git with `.env*`:

    - `.decree/.env`: every script gets it. The process environment wins over it.
    - A machine's `env_file: .env.<name>`: that machine's scripts get it, over `.decree/.env` and
      the process environment. A child machine, router or emitted message runs with its own
      machine's file, never this one's.
    - `.decree/.env.example`: the committed template; decree never reads it.

    Format: `KEY=value` per line, `#` comments, optional `export ` and quotes. Values are
    interpolated as Compose's `env_file` does: `${VAR}`, `$VAR`, `${VAR:-default}`,
    `${VAR-default}`, `$$` for a `$`; single-quoted values stay literal. Write a shared host once:
    `HOST=box` then `COMFY_URL=http://${HOST}:8188`. An unknown variable is empty and
    `decree check` warns. Never source them by hand. `DECREE_*`, `TRACEPARENT` and `TRACESTATE`
    cannot be set in them, except `DECREE_MACHINES` and `DECREE_EXCLUDE_MACHINES` in `.decree/.env`,
    which pick the machines `daemon` run and never reach scripts, and
    `DECREE_MAX_DEPTH` (default 10), the deepest chain of emits and child runs. When two set a name, the first wins: decree's own `DECREE_*`, the
    machine's `env_file`, the process environment, `.decree/.env`.

    One script run by several states with different values: give each state a two-line script,
    `METHOD=image_text exec "$(dirname "$0")/build.sh"`, rather than parsing the state name.

    ## A notify script

    ```bash
    #!/usr/bin/env bash
    # scripts/notify_person.sh: tell someone how to answer. A real one posts to chat or opens an issue.
    set -euo pipefail
    echo "Question: $DECREE_QUESTION"
    cat "$DECREE_CHOICES"
    for event in $DECREE_EVENTS; do
      [ "$event" = error ] && continue
      echo "  decree reply $DECREE_WAIT_ID $event -m \"<note>\""
    done
    ```

    ## Long-running services

    Model servers and similar services are not scripts; run them under systemd or similar. A state
    that needs one starts it in a quick `onentry` script (`systemctl --user start comfyui`), so the
    dependency shows in the machine and its graph. Waiting until it is ready is an invoked state
    with a `timeout`, since `onentry` has none.
cron:
  boot.md: |
    ---
    cron: '@reboot'
    ---
    Install the decree skill.
---
# skill

The decree skill, for an AI CLI that reads skills: `SKILL.md` and `reference/*.md`, which tell an agent how to write messages, machines and scripts and how to run decree. `decree init` adds this plugin and writes the skill for its AI backend; then, each time `decree daemon` starts (its cron file is `@reboot`), `install` copies each file that differs into each skill folder the project has, `.claude/skills/decree/` (Claude Code) and `.github/skills/decree/` (GitHub Copilot). Other files in those folders are the project's and stay.

- **After upgrading decree:** `decree install --official skill`, then `decree daemon --once` (or the next daemon start) writes the new skill.
- **Your own edits:** edit `.decree/machines/skill/scripts/skill/`, then `decree export skill`; a file edited only in `.claude/skills/decree/` is written over at the next start.
- **No skill:** delete `.decree/plugins/skill.md`, then `decree install`.
