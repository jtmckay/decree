---
machine: develop
---
# A09: Plugins: one file per machine, exported, imported and installed

## Overview

Sharing a machine today means copying `machines/<name>.yml` and every file its scripts need: everything in `scripts/<name>/`, plus whichever flat `scripts/` files its states run. The sibling project existential built sharing on top of decree (`src/lib/plugin.sh`, `automation/lib/automations/plugin.sh`). There, a **plugin** is one markdown file whose frontmatter holds a machine and its files. An install writes them into `.decree/`, and an export writes an edited, installed machine back to its file. Only the plugin file is committed. That works, but every project that wants it has to rebuild it in shell with yq. decree should own the format, so that a machine is shared as one file and a project's machines are committed as plugins.

What existential has that decree leaves out:
- flows, `project:` tags, `endpoints:`, `handlers:` and `when:`;
- `cron:`, `store:` and `migrations:`: a project's schedule, data and messages stay its own;
- `lib:`, a folder every plugin writes into: decree has none since A08, and a plugin's files are its scripts directory.

What decree keeps: one file per plugin, `needs:`, install as written with no model asked, export from the installed copy, and never overwriting an edit someone made in place. Plugins write only into `machines/<name>.yml` and `scripts/<name>/`, so two plugins can never write the same file.

Prior art for the name and shape: existential's plugins, Claude Code plugins (one bundle installed into the host's folders) and Home Assistant blueprints (one shareable automation file you import).

## Outcome

### The plugin file

- `.decree/plugins/<name>.md` is one plugin: YAML frontmatter, then a markdown body that tells a person what the plugin is for. Its keys:
  - `name`: equal to the file's stem, matching the machine name pattern.
  - `machine`: the machine, as `machines/<name>.yml` holds it, without `name`.
  - `scripts`: a map of `<path>: <its text>`, each path relative to `scripts/<name>/` and written as it is installed (`implement.sh`, `requirements`, `kinds/command.sh`).
  - `needs`: the other plugins it relies on: a machine it invokes, emits to or names as `router:`.
- An installed file is executable when it starts with `#!`, and otherwise not.
- Paths are plain relative paths: no `..`, no absolute paths.
- `schema/v1/plugin.schema.json` describes the format, `decree schema` writes it, and `schema/v1/CHANGELOG.md` records it.

### Installing

- `process`, `daemon`, `check` and `graph` first install every plugin in `plugins/` that is new or has changed since it was installed, and only then read machines. `process` and `daemon` do this before each run, as they already reload machines. A fresh clone that has only `plugins/` runs without any other step.
- A plugin installs exactly `machines/<name>.yml` and the files under `scripts/<name>/`. A file in `scripts/<name>/` that the plugin does not ship is removed when it is installed, so the directory always matches the plugin.
- A plugin whose file was deleted has its installed files removed.
- What decree records about an install stays under the gitignored `.decree/cache/`.
- Nothing is overwritten when a plugin's installed copy was edited in place. Adding, changing or deleting a file in `scripts/<name>/`, or editing the machine, all count as edits. If the plugin file has not changed, the edit simply stays and runs. If the plugin file changed too (after a `git pull`, say), that plugin is not installed again: the edited copy keeps running, `process` and `daemon` print the conflict once, and `decree check` exits 1 with an error naming the plugin. The error gives both ways out: `decree plugin export <name>` keeps the edit (merge with git), and `decree plugin install <name>` takes the plugin's version. When an installed copy already exists and was never installed by decree (a project converting to plugins), it is treated the same way: if it matches the plugin, it is just recorded; if it differs, it is a conflict.
- A machine or scripts directory that no plugin installed, and the flat `scripts/` files, are never touched. Projects without `plugins/` work exactly as before.
- `decree plugin install [<name>...]` installs the named plugins (or all of them) now, replacing an edited installed copy. This is how an edit is discarded.

### Export and import

- `decree plugin export <name>` writes `plugins/<name>.md` from the installed copy. The output includes:
  - the machine;
  - every file in `scripts/<name>/`;
  - every flat `scripts/` file its states run (an invoke's script, a `person` decision's `ask`, `onentry`, `onexit`), placed in `scripts/` of the plugin so that it installs into `scripts/<name>/`, where decree resolves it first;
  - `needs`: every other machine it invokes, emits to or routes through.
  An existing plugin file keeps its body. A new one gets `# <name>` and the machine's `description`. It works for a machine that was never a plugin, which is how a project converts.
- Export is deterministic, so git diffs show only real changes: installing a plugin and then exporting it gives the same bytes, and exporting the same installed copy twice gives the same file.
- `decree plugin import <file|->` checks a plugin file and copies it to `plugins/<name>.md`, then installs it. Before installing, it prints what the plugin runs: the machine's description, each file with its line count, and `needs`.
  - It refuses a file that fails the schema.
  - It refuses a plugin whose `needs` are not in `plugins/`, naming them.
  - It refuses a name that `plugins/` already has, unless `--force` is given. With `--force` it prints what changes.
  - decree fetches nothing: `curl -fsSL <url> | decree plugin import -` is the way to import from a URL.
- `export`, `import` and `install` take `--format json`, described by `schema/v1/cli/plugin.schema.json`.

### Checks

`decree check` reports each of these as an error with a rule id:
- a plugin that fails the schema;
- a missing `needs`;
- the edit conflict above.

It warns, but does not fail, when a project has `plugins/` and a machine that no plugin installed. The warning names `decree plugin export <name>`.

## Constraints

- decree runs no git command and no network request.
- Machines and scripts installed from a plugin behave exactly as if they had been written by hand: same paths, same `DECREE_*` variables, same resolution order.
- `init`, the built-in machines, the default `.gitignore`, this repository's own `.decree/` and the examples do not change in this migration. A10 moves them onto plugins.
- Only this migration's scope.
- Never edit `.decree/migrations/`, `.decree/runs/` or `.decree/processed.md`, not even by a search and replace across the repository: exclude them from every bulk edit.
- If the reference docs and this migration disagree in a way you cannot settle, write the question to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- No test calls a real model or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass, and `decree check` passes in every example directory.
- Documentation:
  - the reference docs gain the plugin format, the commands and where plugins sit in the file layout;
  - the decree skill template gains a rule: plugins are the source, so edit the installed copy, then `decree plugin export <name>`;
  - CHANGELOG lists the change under Added;
  - `docs/decisions.md` gets a decision citing existential, Claude Code plugins and Home Assistant blueprints, and saying what was left out and why.

## Acceptance Criteria

- **Given** a project whose `.decree/` holds only `plugins/develop.md`, the plugins it needs, and messages
  **When** `decree process` runs
  **Then** the machine and its files are installed under `machines/` and `scripts/develop/`, and the run behaves as it would with those files written by hand

- **Given** a machine written by hand, with scripts and a data file in `scripts/<name>/` and a script in flat `scripts/` that one of its states runs
  **When** `decree plugin export <name>` runs, the installed files are deleted, and `decree check` runs
  **Then** the plugin file holds all of them; the files are installed again under `scripts/<name>/` with the same bytes, the flat script among them; and the run resolves the same scripts

- **Given** an installed plugin
  **When** `decree plugin export` runs twice
  **Then** both outputs are byte-identical to the plugin file it was installed from

- **Given** an installed plugin whose script was edited in place
  **When** `decree process` runs with the plugin file unchanged
  **Then** the edited script runs and is not overwritten

- **Given** an edited installed copy and a changed plugin file
  **When** `decree check` runs
  **Then** it exits 1 with an error that names the plugin, `decree plugin export <name>` and `decree plugin install <name>`, and the edited copy is untouched

- **Given** that conflict
  **When** `decree plugin install <name>` runs
  **Then** the installed copy is the plugin's, a file added in place to `scripts/<name>/` is gone, and `decree check` passes

- **Given** an installed plugin
  **When** its file is deleted from `plugins/` and `decree check` runs
  **Then** its machine and `scripts/<name>/` are gone, and a machine that no plugin installed is still there

- **Given** a plugin file on stdin whose `needs` names a plugin the project lacks
  **When** `decree plugin import -` runs
  **Then** it exits 1, names the missing plugin and writes nothing

- **Given** a plugin named like one in `plugins/`
  **When** `decree plugin import` runs without `--force`, then with it
  **Then** the first exits 1 and changes nothing, and the second prints what changes and installs it

- **Given** a plugin whose `scripts` has the path `../x.sh`
  **When** `decree check` runs
  **Then** it exits 1 naming the path

- **Given** a project with `plugins/` and a hand-written machine `extra`
  **When** `decree check` runs
  **Then** it exits 0 and warns that `extra` has no plugin, naming `decree plugin export extra`

- **Given** a project with no `plugins/`
  **When** the existing test suite runs
  **Then** it passes unchanged
