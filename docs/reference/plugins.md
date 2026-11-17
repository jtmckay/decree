# Plugins

A plugin is one machine and the files of its scripts folder, shared as one markdown file: `.decree/plugins/<name>.md`. decree installs it into `machines/<name>.yml` and `scripts/<name>/`, where a machine written by hand would sit, so an installed machine runs exactly as if it had been written by hand: same paths, same `DECREE_*` variables, same [resolution](scripts.md#resolution) order. Only the plugin file is committed; a fresh clone that has only `plugins/` runs without any other step. Why: [D78](../decisions.md#d78-a-machine-is-shared-as-one-plugin-file).

## The plugin file

YAML frontmatter, then a markdown body that tells a person what the plugin is for. decree keeps the body as written.

```markdown
---
name: hello
needs:
- router
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
    echo "hi $(cat .decree/scripts/hello/name.txt)"
  name.txt: |
    world
---
# hello

Says hello, then asks a model whether to say it again.
```

| Key | Required | Meaning |
| --- | --- | --- |
| `name` | yes | Equal to the file's stem, and matches `^[a-z][a-z0-9_]*$`. Names the installed `machines/<name>.yml` and `scripts/<name>/`. |
| `machine` | yes | The machine as `machines/<name>.yml` holds it, without `name` ([machines.md](machines.md)). |
| `scripts` | no | A map of `<path>: <its text>`, each path relative to `scripts/<name>/` and written as it is installed (`implement.sh`, `requirements`, `kinds/command.sh`). A path is plain and relative: no `..`, no `.`, no empty part, no leading `/`. |
| `needs` | no | The other plugins it relies on: a machine it invokes, emits to or routes through (`router:`, or the machine `router` that a `model` state with no `router:` uses). |

No other key is allowed: a plugin carries no `cron:`, `store:` or `migrations:`, since a project's schedule, data and messages stay its own. An installed file is executable when it starts with `#!`, and otherwise not. `.decree/schema/v1/plugin.schema.json` describes the frontmatter ([Schemas](README.md#schemas)).

## Installing

`process`, `daemon`, `check` and `graph` first install every plugin in `plugins/` that is new or has changed since it was installed, and only then read machines. `process` and `daemon` do it again before each run, as they reload machines ([Edits while running](cli.md#edits-while-running)).

- A plugin writes exactly `machines/<name>.yml` and the files under `scripts/<name>/`, so two plugins never write the same file. A file in `scripts/<name>/` that the plugin does not ship is removed when it is installed: the folder always matches the plugin.
- When a plugin file is deleted, its installed copy is removed.
- A machine or scripts folder that no plugin installed, and the flat `scripts/` files, are never touched. A project without `plugins/` works as before.
- What decree records about an install is a copy of the plugin file as installed, `.decree/cache/plugins/<name>.md`, under the gitignored `cache/`.

**Edits in place.** Nothing is overwritten when a plugin's installed copy was edited in place. Adding, changing or deleting a file in `scripts/<name>/`, or editing the machine, all count as edits; the machine is compared as YAML, so a comment or the order of its keys is not one. Deleting the whole installed copy (machine and folder) is not an edit: it is installed again, as in a fresh clone.

- If the plugin file has not changed, the edit stays and runs.
- If the plugin file changed too (after a `git pull`, say), the plugin is not installed again: the edited copy keeps running, `process` and `daemon` print the conflict once, and `decree check` exits 1 with a P3 error naming the plugin and both ways out: `decree plugin export <name>` keeps the edit (merge the plugin file with git), and `decree plugin install <name>` takes the plugin's version.
- An installed copy that exists but was never installed by decree (a project converting to plugins) is treated the same way: if it matches the plugin, it is recorded; if it differs, it is a conflict.
- A deleted plugin whose installed copy was edited keeps that copy, which is then a machine no plugin installed.

So the plugin is the source: edit the installed copy, try it, then run `decree plugin export <name>` and commit the plugin file.

## Commands

| Command | Behaviour |
| --- | --- |
| `decree plugin export <name> [--format <text\|json>]` | Writes `plugins/<name>.md` from the installed copy: the machine; every file in `scripts/<name>/`; every flat `scripts/` file its states run (a script invoke, a `person` state's `ask`, `onentry`, `onexit`), placed under `scripts` so that it installs into `scripts/<name>/`, where decree resolves it first (it is also copied there now); and `needs`, every other machine it invokes, emits to or routes through. An existing plugin file keeps its body; a new one gets `# <name>` and the machine's `description`. Works on a machine that was never a plugin, which is how a project converts. Prints the file and what it installs. |
| `decree plugin import <file\|-> [--force] [--format <text\|json>]` | Checks a plugin file (`-` reads stdin), copies it to `plugins/<name>.md` and installs it. Before installing it prints what the plugin runs: the machine's description, each file with its line count, and `needs`. It refuses, exit 1 and writing nothing, a file that fails the schema; a plugin whose `needs` are not in `plugins/`, naming them; and a name that `plugins/` (or a hand-written `machines/<name>.yml`) already has, unless `--force`, with which it prints each file added, changed or removed and installs it over any edit. decree fetches nothing: `curl -fsSL <url> \| decree plugin import -` imports from a URL. |
| `decree plugin install [<name>...] [--format <text\|json>]` | Installs the named plugins, or every one, now, replacing an installed copy that was edited in place. This is how an edit is discarded. Exit 1, installing nothing, if a name has no plugin file or one fails the schema. |

Export is deterministic, so git diffs show only real changes: installing a plugin and exporting it gives the same bytes, and exporting the same installed copy twice gives the same file. Keys come in the order `name`, `needs`, `machine`, `scripts`, the machine's keys as its file has them, and scripts by path. `--format json` prints one document, described by `.decree/schema/v1/cli/plugin.schema.json`.

## Checks

`decree check` reports, each as an error with its rule ([Validation](machines.md#validation)):

- P1: a plugin file that fails the schema (a bad `name`, a `machine` with `name`, a script path such as `../x.sh`, an unknown key);
- P2: a `needs` that is not in `plugins/`;
- P3: a plugin that changed while its installed copy was edited in place.

It warns, without failing, about each machine that no plugin installed in a project that has `plugins/`, naming `decree plugin export <name>`.
