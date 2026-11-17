---
machine: develop
---
# A09: Official plugins, built into decree, all opt-in

## Overview

A08 made plugins decree's way to keep and share a machine. decree also ships machines of its own, in two forms:
- **Built-ins:** `develop`, `infra`, `qa` and `router`. `decree init` writes every one of them into every project, needed or not, as loose files in `machines/`, `scripts/` and `lib/`.
- **Examples:** the newsletter, the ComfyUI machine in text-to-media, and the routers and machine in route-by-complexity. You can only get these by copying files from the repository.

The user wants both to become **official plugins**:
- built into the decree binary, so there is no network access and no version skew;
- all of them opt-in: `init` installs none of them and then prints what is available.

The user also wants a project to commit `.decree/plugins/` and nothing that is installed from it. The default `.gitignore` should ignore `machines/`, `scripts/` and `lib/`, as existential's `.decree/.gitignore` already does.

The user decided that a project's own edits to a plugin live in its plugin file, not in a separate config folder. That covers `gate.sh`, infra's requirements and kinds, and qa's invariants, wrenches and `QA_RUN`. A project edits the installed copy and exports it, and the `commit` script exports for it, so what a run changes is committed with the run. Sharing a project's plugin therefore shares its edits as well; that is accepted.

The verbs follow npm and cargo:
- `add` takes an official plugin into `plugins/`;
- `import` takes a file into `plugins/`;
- `install` writes `plugins/` into `machines/`, `scripts/` and `lib/`;
- `export` writes the installed copy back into `plugins/`.

## Outcome

### The official plugins

- The repository's `plugins/<name>.md` are the official plugins, and decree builds every one of them into its binary. There is one plugin per machine; lib that several of them share is a lib-only plugin, such as `ai` for `lib/ai.sh`. They are:
  - `develop`, `infra`, `qa` and `router`;
  - every machine under `examples/`: `newsletter`, `comfy`, `develop_by_size`, `gliner_router` and `llm_router`;
  - the lib plugins those need.
- Each official plugin's body says, for a person who has never seen decree, what it does, what it needs (services, tools, a cron file to write), and how to use it.
- Each official plugin, added alone with its `needs` into an empty project, passes `decree check`.
- A script that several plugins share (`clean_tree`, `commit`, `keep`) is in each plugin that runs it, and a test keeps the copies identical.
- `examples/` keeps only what is not a plugin, linked from the plugin that uses it: the observability config, the GLiNER server, demo messages and the project walkthrough.

### Listing and adding

- `decree plugin list` lists every official plugin with its one-line description. Each line is marked:
  - **added**, if the project's `plugins/` has it;
  - **differs**, if the project's copy differs from this decree's (edited, or from an older decree).
  `--format json` is described by the plugin CLI schema.
- `decree plugin add <name>...` copies official plugins, and every official plugin they need that is not yet there, into `plugins/`, then installs them. It names each plugin it added because something needed it.
  - A plugin already in `plugins/` is left as it is, and the command says so.
  - `--force` replaces it with this decree's copy and prints what changes, including the project's own edits that it drops.
  - An unknown name exits 1 and lists the official names.
- `decree upgrade`'s after-steps say that `decree plugin list` shows which plugins the new version changed, and that `decree plugin add --force <name>` takes the new copy.

### `decree init`

- `init` installs no plugin. `.decree/plugins/` exists and is empty.
- `init` still never prompts. When it is done, it prints the official plugins as `decree plugin list` does, followed by the line that adds one.
- It still writes the skill, `.env.example` and the `.gitignore`.
- The `--ai` choice no longer has to be baked into installed files. `ai` picks its backend from:
  1. the run's `ai_cli` data;
  2. `AI_CLI`;
  3. otherwise, the first of `claude`, `opencode` and `copilot` found on `PATH`.
  The `router` plugin follows the same order. `init --ai` still chooses the skill and writes `AI_CLI` into `.env.example`.

### The default `.gitignore`

It is the current template plus `machines/`, `scripts/` and `lib/`. `graph/` and `plugins/` stay committed.

### Committing a run's edits

- The `commit` script exports every plugin whose installed copy differs from its plugin file before it commits. A run's edits to installed files (an `infra` run rewriting its requirements, a `develop` run editing its gate) reach `plugins/` in the run's one commit.
- A run that fails exports nothing. `keep` leaves its edits in the installed copy, where a retry builds on them.
- Every doc and prompt that says to edit a plugin's file in place says the edit is kept by `decree plugin export <name>`, or by the next `commit`.

### This repository

- This repository's own `.decree/` keeps its machine names, so queued migrations naming `develop` still run. Its machines are in its `.decree/plugins/`, started from the official ones plus its own edits: the cargo gate, `QA_RUN`, `lib/develop/conventions.md`, and done's `restart` with `lib/develop/run-queue.sh`, which keep the queue running unattended on a decree rebuilt after each migration. `run-queue.sh` still works after the conversion.
- No file under any `.decree/machines/`, `.decree/scripts/` or `.decree/lib/` is tracked any more, anywhere outside `tests/fixtures/`.
- The tests that ran the examples' machines (newsletter, text-to-media, route-by-complexity) run the official plugins instead.

### Existing projects

- `docs/reference/` says how an existing project converts:
  1. `decree plugin export <name>` for each machine;
  2. add the three lines to `.decree/.gitignore`;
  3. `git rm -r --cached .decree/machines .decree/scripts .decree/lib`.
- decree never edits an existing `.gitignore`.

## Constraints

- decree runs no git command and no network request.
- The machines keep their states, events and behaviour. Only where their files come from changes.
- `decree check` passes in a fresh clone of this repository, in the root and in every directory under `examples/` that has a `.decree/`, before any other command has run.
- Only this migration's scope.
- Never edit `.decree/migrations/`, `.decree/runs/` or `.decree/processed.md`, not even by a search and replace across the repository: exclude them from every bulk edit.
- If the reference docs and this migration disagree in a way you cannot settle, write the question to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- No test calls a real model or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass, and `decree check` passes in every example directory.
- Documentation:
  - the README quick start, `src/templates/synopsis.md` (the user's text: change it only where it is now wrong) and the decree skill template show `init`, then `decree plugin list` and `decree plugin add develop`;
  - the skill's rules say: commit `plugins/` and `graph/`, never `machines/`, `scripts/` or `lib/`; shared lib belongs to one plugin; the built-in machines section is about official plugins a project may have added;
  - CHANGELOG lists the change under Changed;
  - `docs/decisions.md` gets a decision saying why plugins are opt-in and official ones are built into the binary, and why project edits live in the plugin.

## Acceptance Criteria

- **Given** an empty directory
  **When** `decree init` runs
  **Then** it exits 0 without reading stdin; `.decree/plugins/`, `machines/`, `scripts/` and `lib/` hold no plugin files; its output lists every official plugin and the `decree plugin add` line; and `decree check` passes

- **Given** a fresh `init`
  **When** `.decree/.gitignore` is read
  **Then** it is the previous template plus `machines/`, `scripts/` and `lib/`

- **Given** a fresh `init`
  **When** `decree plugin add develop` runs
  **Then** `plugins/` holds `develop` and every plugin it needs, the output names those it added as needed, and `decree check` passes

- **Given** a project that added `develop` and then edited its gate and exported it
  **When** `decree plugin list` runs, and then `decree plugin add develop` runs without `--force`
  **Then** the list marks `develop` added and differs, and the add changes nothing and says so

- **Given** that project
  **When** `decree plugin add --force develop` runs
  **Then** the output shows the gate edit being dropped, and `plugins/develop.md` equals the built-in copy

- **Given** each official plugin, alone with its `needs`, in an empty project
  **When** `decree check` runs
  **Then** it passes

- **Given** `decree plugin add nope`
  **When** it runs
  **Then** it exits 1 and lists the official names

- **Given** an `init`ed git project with `infra` added, in which an `infra` run (with a stubbed `ai`) adds a requirement line
  **When** the run reaches `done`
  **Then** its commit contains `.decree/plugins/infra.md` with the new line, and nothing under `.decree/lib/`

- **Given** a `develop` run that edits its gate and then fails
  **When** the run ends in `failed`
  **Then** `plugins/develop.md` is unchanged, and the edited gate is still installed

- **Given** this repository after the change
  **When** `git ls-files` runs
  **Then** no path matches `.decree/machines/`, `.decree/scripts/` or `.decree/lib/` outside `tests/fixtures/`, and every `examples/` machine has an official plugin in `plugins/`

- **Given** no `AI_CLI` and no `ai_cli` data, and only `opencode` on `PATH`
  **When** `ai` runs
  **Then** it uses `opencode`
