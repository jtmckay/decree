---
machine: develop
---
# A08: No `lib/`: a machine's code and data live with its scripts

## Overview

`.decree/lib/` and `$DECREE_LIB` are a place for "what scripts share": bash functions, config and data. In practice, almost everything there belongs to one machine: `lib/infra/` is infra's, `lib/qa/` is qa's, the newsletter's feeds, comfy's workflows. The one truly shared file, `lib/ai.sh`, is shared only by the built-ins. The plugins coming next (A09) would each have to own paths in a folder every plugin writes to, and those paths could only ever conflict.

The user wants lib gone as a decree concept. A machine's helpers, config and data sit beside its scripts in `scripts/<machine>/`. decree already resolves only the files a state names there, and ignores the rest. A project that wants a folder of shared code (`lib/`, or any other name) is free to keep one; decree neither knows nor cares.

## Outcome

### What decree stops doing

- decree no longer defines, creates, mentions or exports `DECREE_LIB`, and has no `.decree/lib/` in its layout, its checks, its schemas or its docs. A `DECREE_LIB` in `.decree/.env` is just a variable like any other.
- `decree init` writes nothing into `.decree/lib/`.

### Where the built-ins' files go

- Every file the built-ins kept in `lib/` now sits in the machine's own `scripts/<machine>/`:
  - **`infra`:** `requirements`, `run.sh` and `kinds/`;
  - **`qa`:** `qa.sh`, `invariants.md` and `wrenches.md`;
  - **`ai.sh`:** a copy in each built-in machine's scripts directory that uses it.
- Scripts reach these files by path from the project root, which is already their working directory (for example `.decree/scripts/infra/run.sh`), or from their own location.
- A test keeps the copies of `ai.sh` identical, as for the shared scripts.

### Requirements, checked across machines

- `infra` treats the `requirements` file of every machine's scripts directory (`.decree/scripts/*/requirements`, in name order, `infra`'s own first) as one list. `run.sh check` and `run.sh apply` cover all of them, and each line reported says which file it came from.
- `infra`'s `declare` step edits only `scripts/infra/requirements`.
- `develop`'s `precheck` runs that same check.

### This repository and the examples

- They move the same way. This repository's `lib/` content goes to its machines' scripts directories: `ai.sh`, infra's files, and qa's files with this repository's `QA_RUN`.
- In the examples, the newsletter's feeds and taste, comfy's workflow JSON and route-by-complexity's `ai.sh` move into the scripts directory of the machine that reads them.
- No `.decree/lib/` remains anywhere outside `tests/fixtures/`.

### Docs and the skill

- They no longer mention lib, except for one line in the scripts reference: a project may keep shared code in any folder it likes, and decree does not look at it.
- The skill's rules say that a machine's helpers, config and data go in `scripts/<machine>/`, and that what a machine keeps between runs still goes in `$DECREE_STORE`.

## Constraints

- Every machine keeps its states, events and behaviour. Only where its helper files live changes.
- Script resolution (V12) is unchanged: only a file a state names is ever run by decree, and other files in a scripts directory are never an error.
- `.decree/scripts/develop/run-queue.sh`, `restart.sh` and `conventions.md` keep working, and `run-queue.sh` keeps its path.
- No compatibility shim: CHANGELOG lists the removal under Removed, saying what a project using `$DECREE_LIB` changes (a path from the project root).
- Only this migration's scope.
- Never edit `.decree/migrations/`, `.decree/runs/` or `.decree/processed.md`, not even by a search and replace across the repository: exclude them from every bulk edit.
- If the reference docs and this migration disagree in a way you cannot settle, write the question to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- No test calls a real model or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass, and `decree check` passes in every example directory.
- `docs/decisions.md` gets a decision saying why lib is gone.

## Acceptance Criteria

- **Given** an empty directory
  **When** `decree init` runs
  **Then** there is no `.decree/lib/`, `decree check` passes, and `infra`'s `run.sh check` runs from `.decree/scripts/infra/`

- **Given** any script run by decree
  **When** it prints its environment
  **Then** there is no `DECREE_LIB`

- **Given** `scripts/develop/requirements` naming a command that is not on `PATH`, and `scripts/infra/requirements` naming `git`
  **When** `develop`'s `precheck` runs
  **Then** it fails, naming the missing command and `scripts/develop/requirements`

- **Given** `scripts/develop/` holding `conventions.md`, `ai.sh` and `requirements` beside the scripts its states invoke
  **When** `decree check` runs
  **Then** it passes with no warning about those files

- **Given** this repository
  **When** `git ls-files` runs
  **Then** no path is under a `.decree/lib/` outside `tests/fixtures/`, and the docs (outside `CHANGELOG.md`, `docs/decisions.md` and the one line in the scripts reference) never mention `DECREE_LIB` or `.decree/lib`
