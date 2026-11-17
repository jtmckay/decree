---
machine: develop
---
# A12: Official develop variants: develop_rust, develop_node, develop_react

## Overview

The official `develop` plugin (A09) ships a gate that runs nothing until the project fills it in, and an `infra` whose requirements name only what decree itself needs. Every Rust, Node or React project then writes the same gate and declares the same tools by hand.

The user wants `develop` variants per ecosystem, as official plugins a project adds instead: `decree plugin add develop_rust`. Each variant comes with a gate that already runs that ecosystem's standard checks, and declares the tools it needs. The variants are spelled with `_`, because machine names match `^[a-z][a-z0-9_]*$`.

A variant cannot add lines to `infra`'s own requirements file, which belongs to the `infra` plugin. So `infra` also reads a drop-in directory, `lib/infra/requirements.d/`, as Debian's `sources.list.d` and systemd's `*.d` directories let a package add configuration without editing another package's file.

## Outcome

### The variants

- `develop_rust`, `develop_node` and `develop_react` are official plugins. Each is `develop`'s machine (same states, events and transitions) under its own name, so one project can add two of them (a Rust API and a React front end) and point each message at one.
- Each variant's scripts are identical to `develop`'s, except:
  - the gate;
  - the ecosystem's conventions in the implement, fix and verify prompts: where tests go and which tools to use. This repository's own `develop` already keeps its Rust conventions in `lib/develop/conventions.md`, which its prompts read; that is the starting point for `develop_rust`.
  A test keeps the shared scripts identical across the variants.
- `develop` itself stays as the generic plugin, with its empty gate.

### What each gate runs

Each gate names `pass` or `fail`, as `develop`'s does. A non-zero exit means the checks could not run, for example a missing tool.
- `develop_rust`: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`.
- `develop_node`:
  - it uses the package manager the project's lockfile names (`package-lock.json` → npm, `pnpm-lock.yaml` → pnpm, `yarn.lock` → yarn; npm without a lockfile);
  - it installs dependencies the way that manager does for CI;
  - then it runs `lint`, `typecheck` and `test`, each only if `package.json` defines that script, and says which ones it skipped;
  - with none of them defined, it names `pass` and says it checked nothing.
- `develop_react`: `develop_node`'s checks, then `build`, which a React project must define. Its prompts ask for component tests with the project's own test runner and Testing Library.

### Tools the variants need

- `infra` reads `lib/infra/requirements` and then every file in `lib/infra/requirements.d/`, in name order, as one list. `check` and `apply` cover all of them.
- `infra`'s `declare` step still edits only `lib/infra/requirements`.
- Each variant ships `lib/infra/requirements.d/<variant>` naming its tools: `cargo` for Rust, `node` and `npm` for Node and React (a project using pnpm or yarn adds that line). Each variant `needs` `infra`.

### Listing them

`decree plugin list` (and so `init`'s output) shows the variants, each with a one-line description naming its ecosystem.

## Constraints

- `develop`'s machine and behaviour do not change. This repository's own `.decree/` keeps its `develop` machine as it is.
- Gates run only the project's own scripts and the ecosystem's standard commands. They install nothing globally and never run `sudo`. A missing tool is a non-zero exit that names it.
- Tests put stub `cargo`, `npm`, `pnpm` and `yarn` on `PATH`, recording what they were asked to run. No test needs Node, or network access.
- Only this migration's scope.
- Never edit `.decree/migrations/`, `.decree/runs/` or `.decree/processed.md`, not even by a search and replace across the repository: exclude them from every bulk edit.
- If the reference docs and this migration disagree in a way you cannot settle, write the question to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- No test calls a real model or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass, and `decree check` passes in every example directory.
- Documentation:
  - the decree skill template says to add the variant that fits the project, and that a project in another ecosystem starts from `develop` and fills in its gate;
  - `requirements.d` is documented where `infra`'s requirements are;
  - CHANGELOG lists the change under Added;
  - `docs/decisions.md` gets a decision on variants as separate machines, and on the drop-in directory.

## Acceptance Criteria

- **Given** a fresh `init`
  **When** `decree plugin add develop_rust develop_react` runs
  **Then** both are added with `infra` and the other plugins they need, `decree check` passes, and `decree graph` draws both machines with `develop`'s states

- **Given** `develop_rust`'s gate with a stub `cargo` whose `test` fails
  **When** the gate runs
  **Then** it names `fail` and exits 0, after running fmt, clippy and test in that order

- **Given** `develop_rust`'s gate and no `cargo` on `PATH`
  **When** the gate runs
  **Then** it exits non-zero and names `cargo`

- **Given** a project with `pnpm-lock.yaml` and `package.json` defining only `test`
  **When** `develop_node`'s gate runs with stub `pnpm` and stub `npm`
  **Then** only `pnpm` is called (to install, then `test`), and the output says `lint` and `typecheck` were skipped

- **Given** a `package.json` with no scripts
  **When** `develop_node`'s gate runs
  **Then** it names `pass` and says it checked nothing

- **Given** a React project whose `build` fails
  **When** `develop_react`'s gate runs
  **Then** it names `fail`

- **Given** a project with `develop_node` added
  **When** `lib/infra/run.sh check` runs without `node` on `PATH`
  **Then** it reports `node` missing, read from `lib/infra/requirements.d/develop_node`

- **Given** the variants' plugin files
  **When** the tests run
  **Then** they fail if any script the variants share with `develop` differs between them
