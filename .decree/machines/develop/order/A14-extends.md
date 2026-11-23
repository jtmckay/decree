---
machine: develop
---
# A14: Official plugins are `decree_*` and never modified; a project extends one under its own name; `source` updates a plugin from a URL; `state/` is storage only

## Overview

Today a project customizes an official plugin in one of two places. Both are wrong for code:

- **Editing its file in `plugins/`** (a `gate.sh`, infra's `requirements`). `decree list` then marks the file `differs`, and `decree install --official` on an upgrade replaces the whole file. The project's edit survives only as lines on screen and a file in the archive, which someone puts back by hand.
- **A `*.git.*` file in `state/`** (D87). `lease`'s rules are `state/resources.git.txt`, and its swap scripts are `state/swap/<resource>.git.sh`, which lease runs with bash. This survives an upgrade. But `state/` is the one folder of a machine that its runs, and anything else that writes the project's data, are expected to write. So code that decree executes now sits in the one folder that is meant to be writable.

The second one is what keeps existential (the sibling project, on 0.6.0-rc.2) from using `lease`. Its containers mount `plugins/` and `machines/` read-only, and only the host installs. Each machine's `state/` is a link into the user's workspace, because people edit their data there (feeds, departments). A swap script there would let anything that writes the workspace run code in the automation container, past the only path code takes into that stack: a person reads a plugin file and accepts it.

`needs` does not solve this. It lets plugin A use plugin B (source B's scripts, invoke or emit to B's machine), always through B's fixed folder name. It gives a project no way to change A and keep the change, and no way to have A under another name.

Whether a plugin is official is guessed from its name: `decree list` marks `plugins/develop.md` official because decree ships a `develop`, and `decree install --official` replaces it with decree's copy. A project with a `develop` of its own (existential builds automations from notes with one) would have it archived and replaced by an upgrade, and cannot add the official `develop` beside it. So it forks `develop`, `infra` and `qa` under other names (`repo_develop`, `repo_infra`, `repo_qa`) and maintains the copies. decree does the same inside itself: `develop_rust`, `develop_node` and `develop_react` are three byte-for-byte copies of `develop`'s scripts but a few files, kept identical by a test (D84). `skill` has a smaller version: where it writes is fixed (`.claude/skills/decree/` and `.github/skills/decree/`), so a project whose AI CLI looks elsewhere edits `install.sh`.

The user wants it crystal clear which code is decree's and which is the project's, and what runs. The rules:

- **An official plugin's name starts with `decree_`, and only official plugins may use that prefix.** The name says where the code comes from; nothing is guessed.
- **An official plugin is never modified in a project.** It is always this decree's copy, in `plugins/` and in `machines/`. A project changes what one does by extending it under its own name, or writes its own plugin.
- **Every plugin a project runs is in its `plugins/` in full and installed in full under `machines/<name>/`.** A plugin that another extends or needs is a complete file there and a complete install.
- **A plugin published at a URL says so (`source`), and an update from it is only ever taken with a person seeing it.**
- **`state/` holds data, never code.**

Prior art:
- Reserved prefixes for the platform's own names: Kubernetes' `kubernetes.io/` and `k8s.io/` label and annotation prefixes, `x-` vs registered names, Python's `_` and dunder names, systemd's units under `/usr/lib/systemd/system/` vs the administrator's under `/etc/systemd/system/`.
- Read-only vendor files: `/usr` owned by the package manager and never edited, the administrator's changes in `/etc`; Debian's conffiles and `conf.d/`; systemd drop-ins (`<unit>.d/override.conf`).
- Docker's `FROM`, Compose's `extends`, Kustomize overlays: a file names its base and holds only what it adds or replaces; the base is never edited.
- Go's vendor directory and npm's `package-lock.json` (`resolved`, `integrity`), Cargo.lock's `source` and checksums, `go.sum`, pip's `--require-hashes`: a project records where each dependency came from and pins exactly what it accepted, and a change is refused until a person takes it. decree's committed plugin file is that pin, and readable.
- HTTPS for transport; a pinned copy for a domain that changes hands; review for a publisher that ships something bad. Signatures (apt, Sigstore) prove who published, not what is safe, and are left out.
- The Filesystem Hierarchy Standard: `/var` holds variable data and `/usr` holds programs, and a system may mount `/usr` read-only; systemd's `StateDirectory=` and the common `noexec` mount of data volumes follow the same split.

## Outcome

### Official plugins are `decree_*`

- Every official plugin is renamed with the prefix `decree_`: `decree_ai`, `decree_develop`, `decree_develop_rust`, `decree_lease`, `decree_order`, `decree_skill`, and so on for every file in the repository's `plugins/`, which are renamed `plugins/decree_<name>.md`. Every reference follows: machine names, `needs`, `emits:`, `machine:` invokes, `router:`, sourced paths (`.decree/machines/decree_ai/scripts/ai.sh`), docs, examples, the skill, tests and `decree init` (which adds `decree_ai`, `decree_order`, `decree_prune` and `decree_skill`).
- **Official is the prefix.** A plugin is official exactly when its name starts with `decree_`. `decree list` marks those `official` and every other plugin `own`, whatever decree ships.
- **Nobody else may use the prefix.** A plugin file whose `name` starts with `decree_` but names no plugin this decree ships is a schema error (P1), naming the official plugins and saying the prefix is reserved. So a project cannot write its own `decree_xyz`; adding one to decree's repository `plugins/` and building decree is how an official plugin is made, and then that binary ships it.
- `decree install <name>` of a name `plugins/` has no file for is official only when it has the prefix: `decree install decree_lease`. `decree install lease` with no `plugins/lease.md` exits 1, listing the official names. 0.6 is unreleased: no alias from the old names, and the CHANGELOG says every official plugin was renamed.

### Official plugins are never modified

- `decree install` (with no argument, or naming one) writes each official plugin's file in `plugins/` and its installed copy under `machines/` from this decree, every time. When either is not this decree's copy (edited, or written by an older decree), it is archived first, as any install archives what it would lose (D96), and the lines that change are printed. So after a decree upgrade, `decree install` takes the new version of every official plugin the project has, and adds none it does not have.
- `decree check` fails with P3 when an official plugin's file in `plugins/`, or its installed copy (machine, `scripts/`), is not this decree's copy, saying that official plugins are not edited, that `decree install` restores it (archiving the change), and that changes go in a plugin of the project's own that extends it. After a decree upgrade that is expected until `decree install` runs, which already ends an upgrade (D90). `decree list` shows such a plugin `differs`.
- Cron files, `state/` and `order/` are not covered: cron files stay the project's once installed (D97), and `state/` and `order/` are data.
- `decree export decree_<x>` refuses, exit 1, naming the overlay to write instead (`name: <your name>`, `extends: decree_<x>`). `decree export` with no name skips official plugins and names any whose installed copy was edited, with the same advice.
- `decree install --official` is removed: there is nothing left for it to do.
- An official plugin whose runs change its own scripts cannot keep those changes, so:
  - `decree_infra`'s `declare` writes `requirements` under `.decree/machines/${DECREE_MACHINE}/scripts/`. Run as `decree_infra` itself it fails at once, saying to extend it; a project runs its own plugin that extends `decree_infra`.
  - `decree_develop`'s gate runs nothing and says to extend `decree_develop` with a `scripts/gate.sh` (or to use a variant). A run of a plugin that extends it edits that plugin's own copy, and the run's `commit` exports that plugin.
  - Every other official plugin is checked the same way: anything its body tells a project to edit (qa's invariants, wrenches and `QA_RUN`, infra's kinds) is done in a plugin that extends it, and the body says so.

### `extends`

- A plugin file may have `extends: <plugin>`, naming another plugin in the project's `plugins/`: official, from a URL, or the project's own. Its other keys hold only what it adds to that plugin or replaces in it.
- **Its `name` must not be the base's, and must not start with `decree_`** unless it is an official plugin itself. The base is in `plugins/` in full and installed in full under its own name, so `gpu_lease` extending `decree_lease` leaves both `machines/decree_lease/` and `machines/gpu_lease/` installed, each a complete copy of the code it runs. A same name is P1.
- The base is a need: it must be in `plugins/` (P2 otherwise), and `decree install <name>` of a plugin that extends an official one adds the base as it adds needs (`added decree_develop.md, needed by decree_develop_rust`). Install installs the base first.
- Installing it writes `machines/<name>/` from the base's plugin file first, then from the file, **path by path**:
  - a `scripts`, `cron`, `state` or `order` path in the file replaces the base's file at that path, or adds one;
  - `machine` is one file: if the file has a `machine`, it replaces the base's whole; if not, the base's is installed;
  - `needs` is the base's plus the file's (and see "Meeting a need with another plugin");
  - a path the file maps to `null` removes the base's file at that path (for example `cron: {daily.md: null}`).
- **Schedules run once.** While an installed plugin extends a base, the base is installed with everything but its cron files: the overlay carries them (unless it maps them to `null`), so a schedule never runs twice. Messages addressed to the base still run it. `check` does not warn that the base's cron files are not installed, and removing the overlay installs them again at the next `decree install` (as cron files of a plugin decree has no record of installing, D97).
- The chain is resolved first: a base may itself extend another. A cycle is P1, naming it. A library (a plugin with no machine) cannot be extended: other plugins source it by its folder name, so a copy under another name would never be used (P1).
- When a base changes (a decree upgrade's `decree install`, or a URL update), every plugin that extends it, directly or through a chain, is installed again on the new base.
- `decree list` shows a plugin that extends as `extends <base>`.
- `decree export` of a plugin that extends writes only what differs from what its base installs: each installed file that is not the base's byte for byte, plus `machine` if it is not the base's. So editing an installed copy in place, then exporting, gives the smallest file that installs it, and installing then exporting gives the same bytes. An existing file keeps its `extends`, `source`, body and `x-` keys.
- The archive is unchanged: an edited installed copy is archived as a plugin file before an install replaces it. For an overlay that plugin file is the exported overlay, with its `extends`.

### Meeting a need with another plugin

- An official plugin's scripts never write their own machine's name. They find their own files under `.decree/machines/${DECREE_MACHINE}/`, so they work in a plugin that extends them. Every official plugin with a machine is changed to do so, and a test fails on any script of an official plugin with a machine that names `.decree/machines/<its own name>/`.
- A need can be met by another plugin: a `needs` entry may be a one-key map, `{<need>: <plugin>}`. For example `app_develop`, extending `decree_develop`, with `needs: [{decree_infra: app_infra}]`, where `app_infra` extends `decree_infra`. Installing rewrites the names the mapping covers in the installed `machine.yml`: a `machine:` invoke, a `router:`, and the machines in an `emits:` list. Every script of the plugin gets `DECREE_NEED_<NEED>` (the need's name in capitals) set to the plugin that meets that need. This applies to every need, mapped or not, so `DECREE_NEED_DECREE_INFRA=decree_infra` when nothing maps it.
- The names install rewrites are not the project's edits. Whether the installed `machine.yml` was edited in place is judged against the machine with the mapping applied, and `decree export` writes no `machine` for an overlay whose installed one is the base's with the mapping applied.
- Official scripts that reach another plugin use that variable: `decree_develop`'s `precheck` runs `.decree/machines/${DECREE_NEED_DECREE_INFRA}/scripts/run.sh check`, and `decree_qa` emits to `${DECREE_NEED_DECREE_DEVELOP}`. A test fails on an official script that names another plugin's machine folder, or emits to another plugin, except through `DECREE_NEED_*`.

### `source`: a plugin published at a URL

- A plugin file may have `source: <https URL>`, where it is published. It is the only value `source` takes; an `http://` URL or anything else is P1. With no `source`, nothing updates the plugin. An official plugin has no `source`: it is updated by `decree install` under a new decree.
- `decree install --update [<name>...]` updates each plugin with a `source` (with no name, every one in `plugins/`; a named one with no `source` exits 1, naming it, and writes nothing):
  - fetched with the system `curl` (as `decree upgrade` runs its install line; no new dependency), HTTPS only, following a redirect only to HTTPS on the same host, with a timeout. `DECREE_CURL` names another program to run in its place, for tests.
  - The fetched file must pass the schema, and its `name` and `source` must be the file's own. Otherwise it is refused, naming what differs.
  - When it is the committed file byte for byte, `up to date`. Otherwise it prints the whole diff against the committed file and asks `Take this update to <file>? [y/N]`. Only `y` writes it (archiving the old file, as any replace) and installs it, and every plugin that extends it.
  - With no terminal on stdin, it prints the diff and `not updated: run decree install --update <name> in a terminal to review it`, and writes nothing. No flag skips the prompt.
  - A fetch that fails names the URL and curl's error, and the plugin is left as it is; the others still update. Exit 1 if any failed or was refused.
- `--update` never runs from the daemon or a cron file. `decree install <file|->` is unchanged, and `curl -fsSL <url> | decree install -` remains how a project first adds a plugin from a URL; decree fetches only on `--update`.

### This repository's own `.decree/`

- Its plugins follow the same rules. Each of `.decree/plugins/` that is an official plugin's copy byte for byte becomes `decree_<name>`. Each that differs (`develop`, with this repository's gate, `run-queue.sh` and its other changes; any other) becomes a plugin of the same unprefixed name that extends `decree_<name>` and holds only what differs, with `needs` maps to the overlays it uses. So `.decree/machines/develop/` keeps its name: its `order/` (this queue), `state/order.git.txt` and `scripts/run-queue.sh` stay where they are.
- `decree check` passes in this repository and every example after the change.

### The develop variants extend `decree_develop`

- `decree_develop_rust`, `decree_develop_node` and `decree_develop_react` become `extends: decree_develop` files. Each holds only its own `gate.sh`, its `conventions.md` and its `requirements`, and no `machine`: its installed `machine.yml` is `decree_develop`'s, description included. What the variant is for is said in its body and in the official plugins table in `docs/reference/plugins.md`. `decree list` shows the variant with `decree_develop`'s description and `extends decree_develop`. No rule lets an overlay change part of a `machine`; record that choice in the new decision.
- Adding a variant adds `decree_develop` too, and both are installed.
- The test that keeps their shared scripts identical to `develop`'s is deleted: there are no copies left to compare. D84's consequence "Four copies of `develop`'s scripts ship" no longer holds; record that in the new decision.
- `decree_develop_by_size` is checked for the same: what it shares with `decree_develop` it gets by extending, if it differs only in files; otherwise it stays a plugin of its own and uses `DECREE_MACHINE` and `DECREE_NEED_*` like the rest.

### `decree_lease`: rules and swaps are the project's scripts

- `decree_lease` reads its rules from `scripts/resources`, and runs `scripts/swap/<resource>.sh <from> <to>`. The project supplies both through its own plugin that extends it, and sends lease requests to that plugin's name:

  ```yaml
  ---
  name: gpu_lease
  extends: decree_lease
  scripts:
    resources: |
      gpu  15m  5s  30m  llm,comfy
    swap/gpu.sh: |
      #!/usr/bin/env bash
      ...
  ---
  ```

- The base ships `scripts/resources` with no resources, only the format in its comments, and no swap scripts, as D87 decided. A lease on `decree_lease` itself fails at once, naming the resource and saying to extend it. `state/` keeps only what lease writes (`leases/`, `status/`), and its `state_files:` say so.
- A project still holding `state/resources.git.txt` or `state/swap/` under `machines/lease/` (rc.1 and rc.2 wrote them) is told so by `decree check` with a warning, which names the move, says that requests then go to the new plugin's name, and quotes the files as a ready plugin file `name: local_lease`, `extends: decree_lease`. Nothing reads them. 0.6 is unreleased, so no compatibility path beyond that warning.
- `decree_registry`'s `state/services.git.txt` is data that decree never executes, so it stays where it is.

### `decree_skill`: where it writes is the project's

- Its `install.sh` writes to each folder `scripts/dirs` lists, one per line, relative to the project root or absolute, `#` for comments. With no `scripts/dirs`, it behaves as today. An empty `scripts/dirs` means it writes nowhere, and says so.
- So a project whose AI CLI reads skills from somewhere else adds its own plugin that extends `decree_skill` with its own `dirs`. Its cron file runs at start; `decree_skill`'s does not (Schedules run once).

### `state/` is storage, never code

decree cannot make this airtight: a script can always `bash` or `source` whatever it reads. The rule is stated everywhere a person or an agent learns where files go, and `decree check` catches what it can see.

- **The rule**, in `docs/reference/scripts.md` (State) and `docs/reference/machines.md` (State): a machine's `state/` holds data its runs and people write: records, queues, the project's content. Nothing in it is run, sourced, `eval`ed or loaded as code by decree or by any script. Code and settings that choose what runs (rules like lease's, a gate, a list of folders to write) are scripts: a plugin ships them in `scripts/`, and a project changes an official plugin's with its own plugin that extends it. Say why: `state/` is the folder runs and people are expected to write, and may be writable where `machines/` is mounted read-only, so code there runs without anyone having read it as a plugin file. Replace the sentence in scripts.md that says lease keeps its rules and swap scripts in `state/`.
- **`decree check` warns**, without failing, as the other state warnings do:
  - for each file under a machine's `state/` that has an execute bit set or starts with `#!`, naming the file and saying that `state/` is for data and code belongs in `scripts/`;
  - for each script line of an installed machine's `scripts/` that runs or sources a path under `$DECREE_STATE_DIR` or `state/`: `bash`, `sh`, `source`, `.`, `exec`, `eval "$(cat`, or the path itself as a command. This is a heuristic over the text, documented as one; it names the file and line.
- **The official plugins follow the rule**: a test fails when an official plugin's `state:` seeds a file with an execute bit or a `#!` line, or when its scripts run or source a path under `state/` (the same heuristic).
- **The decree skill** says it in the section on where files go: data in `state/`, code in `scripts/`, never code in `state/`. It also says: official plugins are `decree_*` and never edited; to change one, a plugin of the project's own that extends it; a project never names a plugin `decree_*`; and `source` with `decree install --update` for a plugin from a URL.

## Constraints

- Nothing installs but `decree install`, as D98 decided. `extends` and `source` change what an install writes, not when one happens.
- A plugin that is not official and uses neither `extends` nor `source` installs, exports and archives exactly as today. Tests change only for the rename, `--official`, editing official plugins, lease's rules, skill's folders and the variants' copies.
- New: the reserved prefix, P3, the keys `extends` and `source`, the map form of a `needs` entry, the `DECREE_NEED_*` and `DECREE_CURL` variables, and `--update` in place of `--official`. No new commands. The `state/` checks are warnings, not new rule ids.
- decree makes a network call only in `decree install --update`, through `curl`, and writes what it fetched only after a `y`. Update the "decree fetches nothing" line in `docs/reference/plugins.md` to say exactly that.
- Tests use stub swap scripts, a stub `DECREE_CURL`, and temp projects. No test calls a real model or the network. No new dependencies.
- Only this order's scope. existential is not changed.
- If the reference docs and this order disagree in a way you cannot settle, write the question to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass, and `decree check` passes in this repository and every example directory.
- Documentation:
  - `docs/reference/plugins.md` documents the prefix and that official plugins are never modified (P3), `extends` (own name, base installed, schedules once), the `needs` map and `DECREE_NEED_*`, and `source` with `--update` (its prompt and what it refuses). Its line "A project's own changes to a plugin … live in its plugin file" says how: its own plugin that extends the official one. The official plugins table uses the new names.
  - `docs/reference/cli.md` removes `--official`, adds `--update`, and `decree upgrade`'s text says `decree install` takes the new official plugins.
  - `docs/reference/scripts.md` and `docs/reference/machines.md` state the `state/` rule and its warnings, and the env var table lists `DECREE_NEED_<NEED>` and `DECREE_CURL`.
  - The `decree_lease`, `decree_skill`, `decree_infra` and `decree_develop*` bodies show the extending file.
  - The decree skill covers what is listed above under "The decree skill".
  - The CHANGELOG lists `extends`, `source`, `--update`, P3 and the `state/` warnings under Added. It lists under Changed the rename to `decree_*`, that official plugins are not edited, the move of lease's rules and swap scripts, and the variants becoming overlays. It lists `--official` under Removed.
  - `docs/decisions.md` gets a decision citing the prior art above. It supersedes D87's "swap scripts live in `state/`", D84's copies, D97's `--official`, and every earlier decision's guidance to edit an official plugin's file (D77, D86 and any other: find them); and records that a variant takes `decree_develop`'s machine whole, that an overlay never shares its base's name, that a base's schedules move to its overlay, and how a URL update is fetched and accepted.

## Acceptance Criteria

- **Given** a new project
  **When** `decree init` then `decree install decree_develop` run
  **Then** `plugins/` holds `decree_ai.md`, `decree_order.md`, `decree_prune.md`, `decree_skill.md` and `decree_develop.md`, each this decree's copy byte for byte, and `decree list` shows each `official`

- **Given** a project with a plugin `decree_mine` (not shipped by decree), or a plugin `app` with `extends: decree_lease` and `name: decree_app`
  **When** `decree check` runs, and when `decree install <file>` is given it
  **Then** both fail with P1, saying the prefix is reserved and listing the official plugins, and nothing is written

- **Given** a project whose own `plugins/develop.md` has no prefix, with a different machine than `decree_develop`'s
  **When** `decree list` and `decree install` run
  **Then** it is shown `own`, the file and `machines/develop/` are unchanged and nothing is archived, and `decree install decree_develop` adds the official one beside it

- **Given** an installed project with `plugins/decree_develop.md` edited, and one with `machines/decree_develop/scripts/implement.sh` edited
  **When** `decree check`, then `decree export decree_develop`, then `decree install` run
  **Then** check fails with P3 naming the file and the overlay to write; export exits 1 with the same advice and writes nothing; install archives the edit, prints what changed, and restores this decree's copy

- **Given** a project with `decree_develop` and its own `app_develop` holding `extends: decree_develop` and a `scripts/gate.sh`
  **When** `decree install` runs
  **Then** `machines/decree_develop/` is the official one and `machines/app_develop/` is the official one with that `gate.sh`, and `decree list` shows `app_develop` as `own`, `extends decree_develop`

- **Given** that project, under a decree whose built-in `decree_develop` differs in `implement.sh`
  **When** `decree install` runs
  **Then** `plugins/decree_develop.md` is replaced with the old file archived and the change printed, both `machines/decree_develop/` and `machines/app_develop/` have the new `implement.sh`, `app_develop`'s `gate.sh` is kept, and `plugins/app_develop.md` is unchanged

- **Given** that installed project, with `machines/app_develop/scripts/gate.sh` edited in place
  **When** `decree export app_develop` runs
  **Then** `plugins/app_develop.md` holds `extends: decree_develop` and only the edited `gate.sh`, and installing then exporting it again gives the same bytes

- **Given** a plugin with `name: decree_lease` and `extends: decree_lease`, a plugin extending a library, or a chain of `extends` that loops
  **When** `decree check` runs
  **Then** it fails with P1, naming the problem; and a plugin extending one not in `plugins/` fails with P2

- **Given** a plugin with `source: http://example.com/x.md`, or `source: official`
  **When** `decree check` runs
  **Then** it fails with P1, naming the problem

- **Given** a plugin with `source: https://example.com/x.md` and a stub `DECREE_CURL` that prints a changed copy
  **When** `decree install --update x` runs with `n` on a terminal, then with `y`, then with stdin not a terminal and a further change
  **Then** the first prints the diff and writes nothing; the second writes and installs it and archives the old file; the third prints the diff and the `not updated` line and writes nothing. A stub that prints a file with another `name` or `source`, or one that fails the schema, is refused and nothing is written

- **Given** a project with its own `develop` (no prefix), `app_develop` extending `decree_develop` with `needs: [{decree_infra: app_infra}]`, and `app_infra` extending `decree_infra`
  **When** all are installed and a message for `app_develop` runs with stub AI and a stub gate
  **Then** `precheck` runs `machines/app_infra/scripts/run.sh`, nothing under `machines/develop/`, `machines/decree_develop/` or `machines/decree_infra/` is written, and `decree check` passes

- **Given** `app_qa` extending `decree_qa` with `needs: [{decree_develop: app_develop}]`
  **When** it is installed, exported, installed and exported again, and qa hands on a bug
  **Then** both exports are the same bytes with no `machine` key, nothing is archived, `decree list` shows neither `edited` nor `differs`, and the bug's message is for `app_develop`

- **Given** a message for `decree_infra` itself
  **When** it runs
  **Then** it fails at once, saying to extend `decree_infra`, and nothing under `machines/decree_infra/` changes

- **Given** `gpu_lease` with `extends: decree_lease`, a `scripts/resources` line `gpu 3s 1s 4s` and a stub `scripts/swap/gpu.sh`, with `machines/` made read-only after the install
  **When** two leases for different modes run on `gpu_lease`
  **Then** both are granted in turn, the stub records `"" -> x` and `x -> y`, and nothing is written outside `machines/gpu_lease/state/`; and a lease on `decree_lease` itself fails, saying to extend it

- **Given** a project with `machines/lease/state/swap/gpu.git.sh`
  **When** `decree check` runs
  **Then** it warns that nothing reads it, and prints the `local_lease` file extending `decree_lease` that would

- **Given** a machine whose `state/` holds a file with `#!/bin/sh` and one with its execute bit set, and whose `scripts/run.sh` has the line `source "$DECREE_STATE_DIR/hook.sh"`
  **When** `decree check` runs
  **Then** it exits 0 and warns about each of the three, naming the file (and the line, for the script), and a project of only official plugins gets none of these warnings

- **Given** `decree_skill` and the project's `my_skill` with `extends: decree_skill` and `scripts/dirs` naming an absolute folder outside the project
  **When** `decree install` then `decree daemon --once` run
  **Then** `machines/decree_skill/` is installed without its cron file, and the skill is written to that folder and nowhere under the project root

- **Given** an empty `scripts/dirs`
  **When** the skill's `install.sh` runs
  **Then** it writes nothing and says so

- **Given** the official plugins
  **When** the tests run
  **Then** each is named `decree_*`; no script of one with a machine names `.decree/machines/<its own name>/`, or another plugin's folder, except through `DECREE_MACHINE` and `DECREE_NEED_*`; and none seeds an executable file in `state/` or runs or sources a path under `state/`

- **Given** `decree_develop_rust`
  **When** its plugin file is read, and it is added alone to an empty project with `decree install decree_develop_rust`
  **Then** it is `extends: decree_develop`, with its gate, `conventions.md` and `requirements` and no `machine`; the install adds `decree_develop` too, `decree check` passes, and its installed `machine.yml` and scripts other than those three are `decree_develop`'s

- **Given** this repository
  **When** `decree check` and `decree list` run after the change
  **Then** check passes, `develop` is shown `own`, `extends decree_develop` (or a variant), and `.decree/machines/develop/order/` and `scripts/run-queue.sh` are where they were
