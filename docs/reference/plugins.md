# Plugins

A plugin is one machine's folder shared as one markdown file, `.decree/plugins/<name>.md`: the machine, the files of its scripts folder, its cron files, and seeds for its order files and its state. decree installs it into `.decree/machines/<name>/` (`machine.yml`, `scripts/`, `cron/`, `order/`, `state/`), where files written by hand would sit, so an installed machine runs exactly as if it had been written by hand: same paths, same `DECREE_*` variables, same [resolution](scripts.md#resolution). Every plugin is opt-in: nothing is installed until a project puts its file in `plugins/`, and nothing updates it but a change to that file. The plugin file is committed and its installed copy is not: `decree init`'s `.gitignore` ignores `machines/**` but each machine's `order/`, `graph.md` and `*.git.*` files ([D86](../decisions.md#d86-machines-own-their-folders-order-replaces-migrations)). decree installs a plugin only when asked, by `decree install`; every other command reads the installed copy, and `daemon` and `check` say which plugins are not installed as their files say. So in a fresh clone, after a pull that changes a plugin, or after upgrading decree, run `decree install`, or start the daemon with `decree install && decree daemon` ([Installing](#installing)).

Which code is decree's and which is the project's is in the names: an official plugin's name starts with `decree_`, and only an official plugin's does; it is always this decree's copy, never edited in a project ([Official plugins](#official-plugins)). A project changes what one does with a plugin of its own that extends it ([`extends`](#extends)), and every plugin a project runs is in its `plugins/` in full and installed in full under `machines/<name>/`. Why: [D99](../decisions.md#d99-official-plugins-are-decree_-and-never-edited-a-project-extends-one). Why: [D78](../decisions.md#d78-a-machine-is-shared-as-one-plugin-file), [D85](../decisions.md#d85-a-plugin-carries-any-of-decrees-files-and-installs-only-when-asked), [D86](../decisions.md#d86-machines-own-their-folders-order-replaces-migrations).

## The plugin file

YAML frontmatter, then a markdown body that tells a person what the plugin is for. decree keeps the body as written.

```markdown
---
name: hello
needs:
- decree_router
machine:
  description: Says hello, then asks a model whether to say it again.
  initial: greet
  states:
    greet:
      invoke: greet
      transitions:
        done: again
    # ...
scripts:
  greet.sh: |
    #!/usr/bin/env bash
    echo "hi $(cat "${DECREE_STATE_DIR}/name.txt")"
cron:
  morning.md: |
    ---
    cron: 0 9 * * *
    machine: hello
    ---
    Good morning.
state:
  name.txt: |
    world
---
# hello

Says hello, then asks a model whether to say it again.
```

| Key | Required | Meaning |
| --- | --- | --- |
| `name` | yes | Equal to the file's stem, and matches `^[a-z][a-z0-9_]*$`. Names the folder it installs into, `machines/<name>/`. A name starting with `decree_` is an official plugin's: one that names no plugin this decree ships fails P1. |
| `extends` | no | The plugin it is installed on top of, in `plugins/` ([`extends`](#extends)); its other keys then hold only what it adds or replaces. |
| `source` | no | The `https://` URL it is published at, which `decree install --update` fetches ([`source`](#source-a-plugin-published-at-a-url)). No other value; an official plugin has none. |
| `machine` | no | The machine as `machines/<name>/machine.yml` holds it, without `name` ([machines.md](machines.md)). A plugin without one installs no machine: a plugin of scripts is a library (below). |
| `scripts` | no | A map of `<path>: <its text>`, each path relative to `machines/<name>/scripts/` and written as it is installed (`implement.sh`, `requirements`, `kinds/command.sh`). A path is plain and relative: no `..`, no `.`, no empty part, no leading `/`. |
| `cron` | no | A map of `<file>.md: <its text>`, each a [cron file](messages.md#cron-files) installed as `machines/<name>/cron/<file>.md`, whose `machine:` defaults to `<name>`. A file name, no `/`. |
| `order` | no | A map of `<file>.md: <its text>`, each an [order file](messages.md#order-files-run-in-order-at-each-start) written to `machines/<name>/order/<file>.md` once. A file name, no `/`; it sorts among the project's order files by name, so prefix it as the project does (`A01-newsletter-setup.md`). |
| `state` | no | A map of `<path>: <its text>`, seeds for `machines/<name>/state/` (`$DECREE_STATE_DIR`), each written once. A plain relative path, as in `scripts`. |
| `needs` | no | The other plugins it relies on: a machine it invokes, emits to or routes through (`router:`, or the machine `decree_router` that a `model` state with no `router:` uses), or a library whose scripts it sources. An entry is a name, or a one-key map `{<need>: <plugin>}` naming another plugin that meets that need ([Meeting a need with another plugin](#meeting-a-need-with-another-plugin)). |
| `x-<anything>` | no | Ignored by decree and kept as written by export: room for a tool built on decree (existential's `x-exist-services`), as Compose and OpenAPI keep `x-` keys. |

No other key is allowed. In a plugin that `extends` another, a `scripts`, `cron`, `state` or `order` path may map to `null`, removing the base's file there. An installed script is executable when it starts with `#!`, and otherwise not. `.decree/schema/v1/plugin.schema.json` describes the frontmatter ([Schemas](README.md#schemas)).

**What each key's files become.** `machine` and `scripts` are the plugin's installed copy: installing makes them exactly what the plugin says, an edit to them in place is an edit (below), and they go when the plugin goes. `cron` files are installed with them the first time, and are then the project's: an install of a plugin already installed leaves `cron/` as it is, so a schedule changed or a cron file deleted in place stays, until `decree install --cron` takes the plugin's (below); they go when the plugin goes. `order` and `state` are seeds, written once, never overwritten. `state` seeds data, never code: nothing in a machine's `state/` is run or sourced ([State](scripts.md#state)).

- An order file or state file is written only when it is absent. After that it is the project's: an order file is committed with its history as any other, and the state folder is the project's data. A plugin never overwrites or removes either, even when the plugin goes.

**Libraries.** A plugin with no `machine` installs only `machines/<name>/scripts/` (and any cron files and seeds). Another plugin that uses its scripts lists it in `needs`, so `decree check` (P2) and `decree install` refuse to go without it, and sources it by its installed path from the project root, every script's working directory, through the variable decree sets for the need: `source ".decree/machines/${DECREE_NEED_<NAME>}/scripts/<file>"` (`DECREE_NEED_DECREE_AI` for `decree_ai`). The official `decree_order` plugin is one: its `emit_if_changed.sh` and `emit_if_changed_git.sh` are for any machine. Coupling plugins is explicit: the `needs` entry and the path, nothing implicit. A library is changed like any plugin of the project's own, by changing its file, and every plugin that sources it gets the change. A library cannot be extended: other plugins source it by its folder name, so a copy under another name would never be used; a project that wants another library writes its own and maps the need to it.

## Official plugins are `decree_*`, and never edited

An official plugin is one decree ships, and its name is the only thing that says so: it starts with `decree_` (`decree_develop`, `decree_lease`), and no other plugin may use the prefix, as `kubernetes.io/` labels are Kubernetes' own. A plugin file whose `name` starts with `decree_` but names no plugin this decree ships fails P1, which names the official plugins; an official plugin is made by adding it to decree's repository `plugins/` and building decree.

An official plugin is never modified in a project. Its file in `plugins/` and its installed copy under `machines/<name>/` (`machine.yml`, `scripts/`) are this decree's copy, as `/usr` is the package manager's:

- `decree install` writes each official plugin's file and installed copy from this decree, every time. When either is not this decree's copy (edited, or written by an older decree), it is archived first, as any install archives what it would lose, and the lines that change are printed. So after a decree upgrade, `decree install` takes the new version of every official plugin the project has, and adds none it does not have.
- `decree check` fails with P3 while an official plugin's file or installed copy is not this decree's copy, saying that official plugins are not edited, that `decree install` restores it (archiving the change), and that changes go in a plugin of the project's own that extends it. After a decree upgrade that is expected until `decree install` runs, which ends an upgrade ([D90](../decisions.md#d90-decree-install-writes-the-schemas-and-ends-an-upgrade)). `decree list` shows such a plugin `differs`.
- Cron files, `state/` and `order/` are not covered: an installed cron file is the project's ([D97](../decisions.md#d97-cron-files-are-installed-once---official-alone-updates-the-official-plugins-a-project-has)), and `state/` and `order/` are data.
- `decree export decree_<x>` refuses, exit 1, naming the plugin to write instead (`name: <your name>`, `extends: decree_<x>`). `decree export` with no name skips official plugins and names any whose installed copy was edited, with the same advice.
- `decree install <file>` of an official plugin that is not this decree's copy refuses, exit 1 (P3).

A project changes what an official plugin does by extending it under its own name (below), or writes its own plugin.

## `extends`

A plugin file may name another plugin in `plugins/` with `extends`: official, from a URL, or the project's own. Its other keys hold only what it adds to that plugin or replaces in it, as a Dockerfile's `FROM`, Compose's `extends` and a Kustomize overlay name their base and never edit it.

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
# gpu_lease

This project's GPU, leased through decree_lease.
```

- **Its `name` is its own.** It must not be the base's (P1), and must not start with `decree_` unless it is an official plugin itself (the `decree_develop` variants are). The base is in `plugins/` in full and installed in full under its own name, so `gpu_lease` extending `decree_lease` leaves both `machines/decree_lease/` and `machines/gpu_lease/` installed, each a complete copy of the code it runs.
- **The base is a need.** It must be in `plugins/` (P2 otherwise, and the plugin is not installed), and `decree install <name>` of a plugin that extends an official one adds the base as it adds needs (`added decree_develop.md, needed by decree_develop_rust`).
- **Installing is path by path.** `machines/<name>/` gets the base's plugin first, then the file: a `scripts`, `cron`, `state` or `order` path in the file replaces the base's file at that path, or adds one; a path mapped to `null` removes the base's file there (`cron: {daily.md: null}`); `machine` is one file, so a `machine` in the file replaces the base's whole, and without one the base's is installed; `needs` is the base's plus the file's.
- **Schedules run once.** While an installed plugin extends a base, the base is installed with everything but its cron files: the overlay carries them (unless it maps them to `null`), so a schedule never runs twice. Messages addressed to the base still run it. `check` does not warn that the base's cron files are not installed, and removing the overlay installs them again at the next `decree install`, as cron files of a plugin decree has no record of installing.
- **The chain.** A base may itself extend another; the chain is resolved first. A loop is P1, naming it. A library (a plugin with no machine) cannot be extended (P1).
- **A base that changes** (a decree upgrade's `decree install`, or a URL update) has every plugin that extends it, directly or through a chain, installed again on the new base.
- **Export** of a plugin that extends writes only what differs from what its base installs: each installed file that is not the base's byte for byte, `null` for each of the base's it lacks, and `machine` if it is not the base's (with the `needs` maps applied, below). So editing an installed copy in place, then exporting, gives the smallest file that installs it, and installing then exporting gives the same bytes. An existing file keeps its `extends`, `source`, `needs`, body and `x-` keys.
- The archive is unchanged: an edited installed copy is archived as a plugin file before an install replaces it; for a plugin that extends, that file is the exported overlay, with its `extends`.
- `decree list` shows it as `extends <base>`, with its base's description when it has no machine of its own.

## Meeting a need with another plugin

An official plugin's scripts never write their own machine's name: they find their own files under `.decree/machines/${DECREE_MACHINE}/`, so they work in a plugin that extends them. And they reach another plugin only through a variable: every script of a plugin gets `DECREE_NEED_<NEED>` (the need's name in capitals) set to the plugin that meets that need, for every need, mapped or not (`DECREE_NEED_DECREE_INFRA=decree_infra` when nothing maps it). `decree_develop`'s `precheck` runs `.decree/machines/${DECREE_NEED_DECREE_INFRA}/scripts/run.sh check`, and `decree_qa` emits to `${DECREE_NEED_DECREE_DEVELOP}`. A script of a machine no plugin file installs gets none, so an official script falls back to the need's own name (`${DECREE_NEED_DECREE_AI:-decree_ai}`).

A need can be met by another plugin: a `needs` entry may be a one-key map, `{<need>: <plugin>}`.

```yaml
---
name: app_develop
extends: decree_develop
needs:
- decree_infra: app_infra
---
```

`app_infra` extends `decree_infra`. Installing `app_develop` rewrites, in its installed `machine.yml`, the names the mapping covers: a `machine:` invoke, a `router:` (and a `model` state with no `router:`, when `decree_router` is mapped), and the machines in an `emits:` list; and its scripts get `DECREE_NEED_DECREE_INFRA=app_infra`. The plugin that meets a need must be in `plugins/` (P2). The names install rewrites are not the project's edits: whether the installed `machine.yml` was edited in place is judged against the machine with the mapping applied, and `decree export` writes no `machine` for a plugin whose installed one is its base's with the mapping applied.

## `source`: a plugin published at a URL

A plugin file may say where it is published, `source: <https URL>`. It is the only value `source` takes: an `http://` URL or anything else is P1. With no `source`, nothing updates the plugin. An official plugin has no `source`: `decree install` under a new decree updates it.

`decree install --update [<name>...]` updates each plugin with a `source` (with no name, every one in `plugins/`; a named one with no `source` exits 1, naming it, and writes nothing):

- It fetches the URL with the system `curl` (as `decree upgrade` runs its install line; no new dependency), HTTPS only, following a redirect only to HTTPS on the same host, with a timeout. `DECREE_CURL` names another program to run in its place, for tests.
- The fetched file must pass the schema, and its `name` and `source` must be the file's own; otherwise it is refused, naming what differs, and nothing is written.
- When it is the committed file byte for byte, it prints `up to date`. Otherwise it prints the whole diff against the committed file and asks `Take this update to <file>? [y/N]`. Only `y` writes it (archiving the old file, as any replace) and installs it, and every plugin that extends it.
- With no terminal on stdin, it prints the diff and `not updated: run decree install --update <name> in a terminal to review it`, and writes nothing. No flag skips the prompt.
- A fetch that fails names the URL and curl's error, and the plugin is left as it is; the others still update. It exits 1 if any fetch failed or was refused.

`--update` never runs from the daemon or a cron file. `decree install <file|->` is unchanged: `curl -fsSL <url> | decree install -` remains how a project first adds a plugin from a URL. The committed plugin file is the pin, readable as it is, as a lockfile's checksum pins what a project accepted: HTTPS is the transport, the pinned copy holds against a domain that changes hands, and reading the diff guards against a publisher that ships something bad. Signatures prove who published, not what is safe, and are left out.

## Installing

Installing is a step a person (or a machine of theirs) takes, never a side effect: a pull or an edit to a plugin file changes nothing that runs until it is installed. `plugins/` is committed and `machines/` (but each machine's `order/`, `graph.md` and `*.git.*` state files) is ignored by git, so after a clone or a pull the step is `decree install`. With no argument it installs every plugin in `plugins/`, whether or not its installed copy already matches, as `npm install` does, and removes the installed copy of each deleted one, exiting 1 only if a plugin file fails its schema. A plugin installed for the first time (decree has no record of installing it, as in a fresh clone) gets its cron files; one installed already keeps the cron files it has, unless `--cron`. `decree install <name>...` installs those, seeding an official one into `plugins/` first when the project has no file of that name. A plugin on stdin, `curl -fsSL <url> | decree install -`, or a file, `decree install <file>`, is copied into `plugins/`, replacing a file of that name, and installed. No change blocks an install: what it would lose is moved to the archive first (below). Nothing else installs a plugin but `init`, which seeds the defaults, which a project drops by deleting their files. decree makes a network call only in `decree install --update`, through `curl`, and writes what it fetched only after a person answers `y`.

`daemon` and `check` install nothing. They compare `plugins/` with what is installed and say what `decree install` would do: `daemon` print each such plugin once, on stderr (``plugins/<name>.md: not installed, or changed since it was installed: run `decree install` ``), and look again before each run, as they reload machines ([Edits while running](cli.md#edits-while-running)); `check` warns about it and reports P1, P2 and P3 as errors. A project whose `machines/` is read-only (a container mounting them from the host) runs as installed, and the host installs. A project without `plugins/` works as before.

Where installing should be automatic (a deploy, a container's start command, a service unit), install, then start: `decree install && decree daemon`, so each start runs the plugins as committed; or a git `post-merge` hook runs `decree install` after each pull. decree never installs on its own: the daemon only reports what an install would change. [D98](../decisions.md#d98-no-plugin-installs-on-daemon-start-the-start-command-does).

- A plugin's installed copy is exactly `machines/<name>/machine.yml` and the files under `machines/<name>/scripts/`, with `machines/<name>/cron/` when it is installed the first time or with `--cron`, so two plugins never write the same file. A file in `scripts/` (or, then, `cron/`) that the plugin does not ship is removed when it is installed: the folders match the plugin.
- Once installed, its cron files are the project's: `decree install` leaves `cron/` as it is, edited or emptied, and a plugin that changes its cron files does not change the installed ones. `daemon` and `check` say when they are not the plugin's (``its installed cron files (machines/<name>/cron/) are not the plugin's``); `decree export <name>` writes them to the plugin, and `decree install --cron [<name>]` takes the plugin's, archiving the installed ones when they were edited.
- Its order and state seeds are written once, as above.
- When a plugin file is deleted, its installed copy is removed; its `state/`, `order/` and `graph.md` stay.
- **A plugin added for a need leaves with the last plugin that needed it**, as `apt autoremove` removes a package installed only as a dependency ([D100](../decisions.md#d100-a-plugin-added-for-a-need-leaves-with-the-last-plugin-that-needed-it)). Each plugin file decree copies into `plugins/` because another plugin extends or needs it (`added <file>, needed by <names>`) is recorded as added for a need, `.decree/cache/plugins/<name>.needed`, and that mark is all the record holds. A plugin is needed when a plugin in `plugins/` extends it or lists it in `needs`; a `needs` map entry that meets the need with another plugin (`{decree_infra: infra}`) needs that other plugin, not the official one. Which plugins need it is worked out from `plugins/` as it is each time, never stored. `decree install` with no argument removes the file and installed copy of each plugin added for a need that no plugin a person asked for needs any more, directly or through others added for a need, so a chain leaves together, printing `removed <file>, no longer needed` for each. An installed copy edited in place is archived first, as every removal is, and `state/` and `order/` stay. `decree install <name>` of one marks it asked for, so it stays from then on, as `apt install` of an automatic package does. A plugin file a person wrote, installed by name or from a file, or one with no mark (from rc.1-rc.3, a fresh clone or a lost cache) is never removed this way, and nothing is while a plugin file fails its schema, since what that one needs is not known. Until then it still runs: `daemon` and `check` name it (``plugins/<name>.md: added for a need, and nothing needs it now: run `decree install` to remove it``) and remove nothing, as for a deleted plugin. `decree list` shows a plugin added for a need `(needed by <names>)`; `--format json` gives `needed_by`, null for every other plugin.
- A machine folder that no plugin file names is never touched.
- What decree records about an install is a copy of the plugin file as installed, `.decree/cache/plugins/<name>.md`, under the gitignored `cache/`.

**Edits in place, and the archive.** An install never refuses over a change and never loses one. Before it writes over or removes an installed copy that is not what decree last installed there (edited in place, by a person or a run, or written by hand before the project had the plugin), it exports that copy as one plugin file, as `decree export` would (its machine, `scripts/` and cron files, and the current plugin file's body, `needs`, seeds and `x-` keys), into `.decree/archive/<name>/<time>.md`, the time in UTC to the millisecond (`20261122T153000.123Z`), so the files sort by time. A plugin file that `decree install <file>`, `--update` or an official plugin's restore replaces is archived the same way, as it was; when both happen and differ, the replaced file is `<time>.replaced.md`. It prints each file, and `decree install .decree/archive/<name>/<time>.md` restores one (archiving what that replaces in turn). A copy that cannot be a plugin file, a `machine.yml` that is not a YAML mapping or a script that is not text, is moved as it is into the folder `.decree/archive/<name>/<time>/`; a machine that is YAML but fails its checks is still archived as a file. Nothing else is archived: an unedited copy is what its recorded plugin file installs, so it can be installed again. `.decree/archive/` holds a `.gitignore` of `*`, as pytest's and ruff's caches do, so git ignores it in a project whose `.gitignore` predates it; decree never deletes it, so delete what is no longer wanted.

Adding, changing or deleting a file in `machines/<name>/scripts/`, or editing `machine.yml`, all count as edits (and in `cron/`, when the install takes the cron files); the machine is compared as YAML, so a comment or the order of its keys is not one. Deleting the whole installed copy is not an edit: it is installed again, as in a fresh clone. Until the next install the edited copy runs; `daemon`, `check` and `decree list` (`edited`) say that `decree install` would archive it. To keep an edit, run `decree export <name>` first; to recover one after, `decree install` its archived file.

So the plugin is the source: edit the installed copy, try it, then run `decree export <name>` and commit the plugin file. A project's own changes to a plugin (a `gate.sh`, `requirements`, infra's kinds, qa's invariants, wrenches and `QA_RUN`, lease's resources and swap scripts) live in its plugin file, not in a separate configuration folder ([D79](../decisions.md#d79-official-plugins-are-built-in-and-opt-in)), so sharing the plugin shares them too: for an official plugin, in its own plugin that extends the official one ([`extends`](#extends)), since the official file is never edited.

**A run's edits.** The `commit` script the official plugins carry (`done`'s `onentry`) runs `decree export` with no name before it commits, so what a run changed in an installed copy (a run of the project's `app_infra` adding a line to its `scripts/requirements`, a run of its `app_develop` editing its gate) reaches `plugins/` and is in the run's one commit; git ignores the installed copy. A run that fails exports nothing: `keep` leaves its edits in the installed copy, where `decree retry` builds on them, and counts them (`decree export --dry-run`) among the changes that make its attention file hold its group, so the next run's `commit` does not take them. A `decree install` before the retry (`decree install && decree daemon` runs one at each start) moves those edits to the archive.

## Commands

| Command | Behaviour |
| --- | --- |
| `decree install [--cron] [--format <text\|json>]` | With no argument, first makes each official plugin's file and installed copy this decree's (archiving what was there, printing `replaced <file> with this decree's copy:` or `restored the installed copy of <file> ...` and the lines that change): how a project takes every plugin a new decree version changed. It adds no plugin the project does not have, not even one a new copy `needs`: `check` reports that need (P2), and `decree install <name>` adds it. Then it installs every plugin in `plugins/` (each that extends another on its base) and removes the installed copy of each deleted plugin, and the file and installed copy of each plugin added for a need that no plugin in `plugins/` needs any more, first archiving each installed copy that was edited in place or written by hand as a plugin file in `.decree/archive/` ([Installing](#installing)). It prints a line naming each archived file, `removed the installed copy of <file>, which was deleted` for each deleted plugin, `removed <file>, no longer needed` for each plugin added for a need that went, then `installed <file>` for each; a plugin that fails the schema, or extends one not in `plugins/`, is not installed, is named on stderr, and the exit code is 1 for one that fails the schema. With `--cron`, plugins already installed take their cron files too ([Installing](#installing)). |
| `decree install <name>... [--cron] [--format <text\|json>]` | Installs each named plugin from `plugins/<name>.md`, moving an installed copy edited in place, or a `machines/<name>/` written by hand, to `.decree/archive/` first, and installs again each plugin that extends one of them. A name `plugins/` has no file for is official only when it has the prefix (`decree install decree_lease`), copied there from this decree first, with every official plugin it extends or needs (and theirs) that `plugins/` does not have. A named official plugin whose file is not this decree's copy is replaced by it, the old file going to the archive, and the output lists the lines dropped (`-`) and added (`+`). Each it copied because another extends or needs it is recorded as added for a need, and `decree install` removes it once nothing needs it; a named plugin is recorded as asked for, so one added for a need earlier stays from then on ([Installing](#installing)). It redraws the graphs and prints `added <file>` for each plugin it copied, `added <file>, needed by <names>` for each it copied because another extends or needs it, a line naming each archived file, then `installed <names>`. A name that is neither in `plugins/` nor official (`decree install lease` with no `plugins/lease.md`), or a plugin file that fails the schema, exits 1, writing nothing, and lists the official names. A need that `plugins/` already has is left alone. |
| `decree install --update [<name>...] [--format <text\|json>]` | Fetches each named plugin, or every one with a `source`, from its source, and takes it only after `y` on a terminal ([`source`](#source-a-plugin-published-at-a-url)). |
| `decree install <file\|-> [--cron] [--format <text\|json>]` | Checks a plugin file (`-` reads stdin), copies it to `plugins/<name>.md`, the name its `name:` key gives, and installs it. Before installing it prints what the plugin runs: the machine's description, each file with its line count, and `needs`. It refuses, exit 1 and writing nothing, a file that fails the schema, and a plugin that needs one neither in `plugins/` nor official, naming it (an official one it needs is seeded first, printing `added <file>, needed by <name>`, and recorded as added for a need). An official plugin that is not this decree's copy is refused (P3), and so is a name with the prefix that decree does not ship (P1). A plugin file of that name is replaced: it prints each file added, changed or removed, and moves the old file, and an installed copy edited in place or written by hand, to `.decree/archive/`. `curl -fsSL <url> \| decree install -` installs from a URL. `decree install` with no argument never reads stdin, so it is safe in a script or a loop. |
| `decree list [--format <text\|json>]` | Lists the plugins in `.decree/plugins/`, one line each with its description, marked `official` when its name has the `decree_` prefix, with `differs` when its file or installed copy is not this decree's copy (P3: the project edited it, or another decree version wrote it), or `own`, every other plugin, whatever decree ships; `extends <base>` for a plugin that extends another; and with its install state when it is not installed as its file says: `not installed` (new or changed since it was installed: `decree install`), `edited` (its installed copy was edited in place or written by hand, and not exported: `decree install` would archive it), `missing base` (it extends a plugin not in `plugins/`) or `invalid`. A plugin decree added for a need shows `(needed by <names>)` after its description, the plugins in `plugins/` that extend it or list it in `needs` (`(needed by nothing; ...)` when `decree install` would remove it); `--format json` gives them as `needed_by`, null for a plugin a person asked for. Then the machines in `.decree/machines/` that no plugin installed, and the [official plugins](#official-plugins) the project does not have, each with its description, which `decree install <name>` adds; a note explains each mark shown. |
| `decree export [<name>] [--dry-run] [--format <text\|json>]` | Writes `plugins/<name>.md` from the installed copy: the machine, if there is one; every file in `machines/<name>/scripts/` and `cron/`; and `needs`, every other machine it invokes, emits to or routes through, plus the `needs` the existing plugin file has (a plugin may rely on another in a way its machine does not show: `decree_develop`'s `precheck` runs `decree_infra`'s `run.sh`). For a plugin that extends another, only what differs from its base ([`extends`](#extends)). An official plugin is never exported: `decree export decree_<x>` exits 1, naming the plugin to write instead. An existing plugin file keeps its body, its `order`, its `state` and its `x-` keys, which are not read back from `order/` and `state/`; a new one gets `# <name>` and the machine's `description`. Works on a machine that was never a plugin, which is how a project converts. Prints the file and what it installs. Without a name it exports every plugin of the project's own whose installed copy differs from what its file installs (edited in place, by a person or a run), and prints `wrote <file>` for each, and `not exported: the installed copy of official plugin ...` for an official one; `--dry-run` prints `would write <file>` and writes nothing. A conflict, an edited copy whose plugin file changed since it was installed (a pull), is never exported this way, since that would write the old version over the new file: it prints `not exported: <file> ...` and is left to merge by hand (or `decree install`, which archives the edit). |


Export is deterministic, so git diffs show only real changes: installing a plugin and exporting it gives the same bytes, and exporting the same installed copy twice gives the same file. Keys come in the order `name`, `extends`, `source`, `needs`, `machine`, `scripts`, `cron`, `state`, `order`, then the `x-` keys as written; the machine's keys as its file has them, and each map's files by path. `--format json` prints one document, described by `.decree/schema/v1/cli/plugin.schema.json`: `action` is `sync` for `decree install` with no argument, `install` with names, `import` for a file or stdin, `update` for `--update`, and `list` or `export`.

## Official plugins

decree is built with its official plugins: the decree repository's [`plugins/`](../../plugins/), each compiled into the binary, so adding one needs no network and its version is the binary's. Each name starts with `decree_` ([Official plugins are `decree_*`](#official-plugins-are-decree_-and-never-edited)). `decree init` adds `decree_ai`, `decree_order`, `decree_prune` and `decree_skill`, which a project drops by deleting their files; the others are opt-in: `decree list` shows them and `decree install <name>` adds one ([D86](../decisions.md#d86-machines-own-their-folders-order-replaces-migrations)).

| Plugin | What it does | Extends or needs |
| --- | --- | --- |
| [`decree_develop`](../../plugins/decree_develop.md) | Implements a message with an AI agent, runs the project's gate, has the agent fix a failing gate, then check the acceptance criteria; one commit per run. Its own gate runs nothing: a project extends it with its gate (`app_develop`), or uses a variant. | `decree_ai`, `decree_infra` (its `precheck` runs that plugin's `run.sh check`), `decree_order` (its `commit` records an order file) |
| [`decree_develop_rust`](../../plugins/decree_develop_rust.md) | `decree_develop` for a Rust project: extends it with a gate that runs `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test`, Rust conventions its prompts follow, and `cargo` in its requirements. | extends `decree_develop` |
| [`decree_develop_node`](../../plugins/decree_develop_node.md) | `decree_develop` for a Node project: extends it with a gate that installs with the lockfile's package manager (npm, pnpm or yarn) as CI does, then runs `lint`, `typecheck` and `test` where `package.json` defines them. | extends `decree_develop` |
| [`decree_develop_react`](../../plugins/decree_develop_react.md) | `decree_develop` for a React project: `decree_develop_node`'s gate, then `build`; its conventions ask for component tests with Testing Library. | extends `decree_develop` |
| [`decree_infra`](../../plugins/decree_infra.md) | Declares what the environment needs in a machine's `scripts/requirements` and installs or checks it ([Declared environment](scripts.md#declared-environment)); a project runs its own plugin that extends it (`app_infra`), whose requirements are the project's, and a message for `decree_infra` itself fails at once. | `decree_ai`, `decree_order` |
| [`decree_qa`](../../plugins/decree_qa.md) | Attacks the project and hands each real bug to the plugin that meets its `decree_develop` need as a failing test; a project's plugin that extends it holds `QA_RUN`, its invariants and wrenches. | `decree_ai`, `decree_develop`, `decree_order` |
| [`decree_router`](../../plugins/decree_router.md) | The default router a `model` state with no `router:` asks ([The default router](runs.md#the-default-router)). | |
| [`decree_newsletter`](../../plugins/decree_newsletter.md) | Feeds in, picked by a local model, one markdown issue out, on a cron schedule; a project's plugin that extends it holds its feeds and taste. | |
| [`decree_comfy`](../../plugins/decree_comfy.md) | Renders images and videos on ComfyUI ([text-to-media](../../examples/text-to-media/README.md)). | |
| [`decree_develop_by_size`](../../plugins/decree_develop_by_size.md) | Sizes a change with a classifier, then implements it locally first or with Claude ([route-by-complexity](../../examples/route-by-complexity/README.md)). Its machine is its own, so it uses `DECREE_MACHINE` and `DECREE_NEED_*` rather than extending `decree_develop`. | `decree_ai`, `decree_gliner_router` |
| [`decree_gliner_router`](../../plugins/decree_gliner_router.md) | A typed router: the GLiNER2.5-Decide classifier. | |
| [`decree_llm_router`](../../plugins/decree_llm_router.md) | A typed router: a local model held to the request's `reply_schema`. | |
| [`decree_ai`](../../plugins/decree_ai.md) | A library, no machine: `ai.sh`, the `ai` function the official plugins ask an AI CLI through. Added by `decree init`. | |
| [`decree_order`](../../plugins/decree_order.md) | Runs each machine's `order/` files at each daemon start, in file-name order, each once and again when it changes, recording each in that machine's `state/order.git.txt` ([Order files](messages.md#order-files-run-in-order-at-each-start)). Also a library: `emit_if_changed.sh` and `emit_if_changed_git.sh` queue a message only when it differs from the last one. Added by `decree init`. | |
| [`decree_prune`](../../plugins/decree_prune.md) | Each week, deletes the folders of finished runs older than `PRUNE_OLDER_THAN` (`30d`), but a child whose parent is not finished and a run a process holds. Added by `decree init`. | |
| [`decree_skill`](../../plugins/decree_skill.md) | The decree skill for Claude Code and Copilot: `decree init` writes it, and each daemon start copies each file that differs into `.claude/skills/decree/` and `.github/skills/decree/`, or each folder `scripts/dirs` lists in a plugin that extends it. Added by `decree init`. | |
| [`decree_lease`](../../plugins/decree_lease.md) | One mode per resource (a machine, a model server, a paid API) at a time, held by one lease or up to the resource's `holders` for a limited time, the next picked to keep mode swaps rare ([D83](../decisions.md#d83-leases-are-an-official-plugin-not-core)). What can be leased (`scripts/resources`) and how each resource swaps mode (`scripts/swap/<resource>.sh`) are the scripts of the project's own plugin that extends it, which leases are sent to ([D99](../decisions.md#d99-official-plugins-are-decree_-and-never-edited-a-project-extends-one)). | |
| [`decree_registry`](../../plugins/decree_registry.md) | Where each service is and which lease to get before using it (`state/services.git.txt`, data decree never runs), probed at each daemon start and every five minutes; `registry.sh resolve <service>` gives each place, live ones first. It never takes a lease. | |

Each one's body says, for someone new to decree, what it does, what it needs (services, tools, a cron file to write) and how to use it, and what a project changes in a plugin of its own that extends it; and each one, added alone with what it extends and needs to an empty project, passes `decree check`. Every plugin carries every file its scripts use ([D77](../decisions.md#d77-a-machines-files-live-with-its-scripts)), so a file several use (`clean_tree.sh`, `commit.sh`, `keep.sh`) is a copy in each of `decree_develop`, `decree_infra` and `decree_qa`, kept identical by a test; `ai.sh`, which every plugin that asks AI uses, is the `decree_ai` library's, which they list in `needs`. Tests keep every official script finding its own files through `DECREE_MACHINE` and other plugins through `DECREE_NEED_*`, and none seeding or running code in `state/`.

The `decree_develop` variants (`decree_develop_rust`, `decree_develop_node`, `decree_develop_react`) extend `decree_develop`: each holds only its gate, the `conventions.md` its implement, fix and verify prompts follow, and the tools it declares in its `requirements`, which `precheck` checks, and no `machine`, so its installed machine is `decree_develop`'s, description included, and so are its other scripts. Adding a variant adds `decree_develop` too, and both are installed, so one project can add two and point each message at one. A project in another ecosystem extends `decree_develop` with its own gate.

The `decree_ai` library's `ai.sh`, which `decree_develop` (and so its variants), `decree_infra`, `decree_qa` and `decree_develop_by_size` source, and `decree_router`'s `ask.sh`, pick the AI CLI when they run, so no file names a backend: the run's `ai_cli` data, else `AI_CLI`, else the first of `claude`, `opencode` and `copilot` on `PATH` ([The AI helper](scripts.md#the-ai-helper)).

**A new decree version.** The built-in copies change with decree. After an upgrade `decree check` reports P3 and `decree list` marks `differs` each official plugin whose file or installed copy is not this decree's, and `decree install` takes this decree's copy of each, printing the lines that change and archiving the old file in `.decree/archive/<name>/`; every plugin that extends one is installed again on the new base, keeping its own files. A new copy's cron files are not installed over the project's: `check` says when they differ, and `decree install --cron <name>` takes them.

## Converting an existing project

A project from before 0.6 has `machines/<name>.yml`, `scripts/<name>/`, `store/<name>/`, `cron/`, `migrations/` and `processed.md`. To convert ([D86](../decisions.md#d86-machines-own-their-folders-order-replaces-migrations); the CHANGELOG lists each step):

1. Move each `machines/<name>.yml` to `machines/<name>/machine.yml`, `scripts/<name>/` to `machines/<name>/scripts/` (a flat `scripts/<file>` into each machine that runs it), `store/<name>/` to `machines/<name>/state/` (and `store:` to `state_files:`), and each cron file to `machines/<machine>/cron/`.
2. Move `migrations/*.md` into the `order/` of the machine each names, add the `decree_order` plugin (`decree install decree_order`), and write each machine's `state/order.git.txt`, one `<file> <sha256>` line per file `processed.md` lists; then delete `processed.md`.
3. `decree export <name>` for each machine, which writes `plugins/<name>.md`. A machine that was a copy of an official plugin, edited, becomes a plugin of its own that extends `decree_<name>` and holds only its edits (`name: <name>`, `extends: decree_<name>`, then `decree export <name>` writes only what differs); one that was an unedited copy is replaced by `decree install decree_<name>`.
4. Replace `.decree/.gitignore` with the one `decree init` writes (decree never edits an existing one), `git rm -r --cached .decree/machines`, then `git add .decree` and commit.

A fresh clone then has only `plugins/` and the committed order files, records and graphs: run `decree install` once, and again after a pull that changes a plugin (or start the daemon with `decree install && decree daemon`).

## Checks

`decree check` reports, each as an error with its rule ([Validation](machines.md#validation)):

- P1: a plugin file that fails the schema (a bad `name`, a `decree_` name decree does not ship, a `machine` with `name`, a script path such as `../x.sh`, a cron file or order file name with `/` or without `.md`, an unknown key that does not start with `x-`, a `source` that is not an https URL, an `extends` that names the plugin itself, a library, or loops);
- P2: a `needs` or `extends` that is not in `plugins/`;
- P3: an official plugin whose file or installed copy is not this decree's copy.

It warns, without failing, about each machine that no plugin installed in a project that has `plugins/`, naming `decree export <name>`, about each installed copy that `decree install` would move to the archive, and about code in `state/` ([State](scripts.md#state)).
