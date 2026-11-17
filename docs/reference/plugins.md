# Plugins

A plugin is one machine and the files of its scripts folder, shared as one markdown file: `.decree/plugins/<name>.md`. decree installs it into `machines/<name>.yml` and `scripts/<name>/`, where a machine written by hand would sit, so an installed machine runs exactly as if it had been written by hand: same paths, same `DECREE_*` variables, same [resolution](scripts.md#resolution) order. Only the plugin file is committed (`decree init`'s `.gitignore` lists `machines/` and `scripts/`); a fresh clone that has only `plugins/` runs without any other step. Why: [D78](../decisions.md#d78-a-machine-is-shared-as-one-plugin-file).

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

So the plugin is the source: edit the installed copy, try it, then run `decree plugin export <name>` and commit the plugin file. A project's own changes to a plugin (a `gate.sh`, `requirements`, infra's kinds, qa's invariants, wrenches and `QA_RUN`) live in its plugin file, not in a separate configuration folder ([D79](../decisions.md#d79-official-plugins-are-built-in-and-opt-in)), so sharing the plugin shares them too.

**A run's edits.** The `commit` script the official plugins carry (`done`'s `onentry`) runs `decree plugin export` with no name before it commits, so what a run changed in an installed copy (an `infra` run adding a line to `scripts/infra/requirements`, a `develop` run editing its gate) reaches `plugins/` and is in the run's one commit; git ignores `machines/` and `scripts/`. A run that fails exports nothing: `keep` leaves its edits in the installed copy, where `decree process --retry` builds on them, and counts them (`decree plugin export --dry-run`) among the changes that make its attention file hold the queue, so the next run's `commit` does not take them.

## Commands

| Command | Behaviour |
| --- | --- |
| `decree plugin list [--format <text\|json>]` | Lists the [official plugins](#official-plugins) built into this decree, one line each with its machine's `description`, marked `added` when `plugins/` has a plugin of that name and `differs` when that file is not this decree's copy byte for byte (the project edited it, or another decree version wrote it). |
| `decree plugin add <name>... [--force] [--format <text\|json>]` | Copies the named official plugins into `plugins/`, with every official plugin they need (their `needs`, and theirs) that `plugins/` does not have, installs them and redraws `graph/`. It prints `added <file>` for each, and `added <file>, needed by <names>` for each it added because another needs it. A named plugin already in `plugins/` is left as it is: `kept <file>: … nothing changed`. With `--force` it is replaced by this decree's copy, and the output lists each file that changes with the lines dropped (`-`) and added (`+`), so the project's own edits it drops are on screen, and says if the installed copy had edits in place that were not exported. A name that is not an official plugin exits 1, writing nothing, and lists the official names. A need that `plugins/` already has is the project's own and is left alone; a `machines/<name>.yml` that no plugin installed exits 1, naming `decree plugin export <name>`. |
| `decree plugin export [<name>] [--dry-run] [--format <text\|json>]` | Writes `plugins/<name>.md` from the installed copy: the machine; every file in `scripts/<name>/`; every flat `scripts/` file its states run (a script invoke, a `reply` state's `notify`, `onentry`, `onexit`), placed under `scripts` so that it installs into `scripts/<name>/`, where decree resolves it first (it is also copied there now); and `needs`, every other machine it invokes, emits to or routes through, plus the `needs` the existing plugin file has (a plugin may rely on another in a way its machine does not show: `develop`'s `precheck` runs `infra`'s `run.sh`). An existing plugin file keeps its body; a new one gets `# <name>` and the machine's `description`. Works on a machine that was never a plugin, which is how a project converts. Prints the file and what it installs. Without a name it exports every plugin whose installed copy differs from what its file installs (edited in place, by a person or a run), and prints `wrote <file>` for each; `--dry-run` prints `would write <file>` and writes nothing. |
| `decree plugin import <file\|-> [--force] [--format <text\|json>]` | Checks a plugin file (`-` reads stdin), copies it to `plugins/<name>.md` and installs it. Before installing it prints what the plugin runs: the machine's description, each file with its line count, and `needs`. It refuses, exit 1 and writing nothing, a file that fails the schema; a plugin whose `needs` are not in `plugins/`, naming them; and a name that `plugins/` (or a hand-written `machines/<name>.yml`) already has, unless `--force`, with which it prints each file added, changed or removed and installs it over any edit. decree fetches nothing: `curl -fsSL <url> \| decree plugin import -` imports from a URL. |
| `decree plugin install [<name>...] [--format <text\|json>]` | Installs the named plugins, or every one, now, replacing an installed copy that was edited in place. This is how an edit is discarded. Exit 1, installing nothing, if a name has no plugin file or one fails the schema. |

Export is deterministic, so git diffs show only real changes: installing a plugin and exporting it gives the same bytes, and exporting the same installed copy twice gives the same file. Keys come in the order `name`, `needs`, `machine`, `scripts`, the machine's keys as its file has them, and scripts by path. `--format json` prints one document, described by `.decree/schema/v1/cli/plugin.schema.json`.

## Official plugins

decree is built with its official plugins: the decree repository's [`plugins/`](../../plugins/), each compiled into the binary, so adding one needs no network and its version is the binary's. `decree init` adds none of them; `decree plugin list` shows them and `decree plugin add <name>` adds one ([D79](../decisions.md#d79-official-plugins-are-built-in-and-opt-in)).

| Plugin | What it does | Needs |
| --- | --- | --- |
| [`develop`](../../plugins/develop.md) | Implements a message with an AI agent, runs the project's gate, has the agent fix a failing gate, then check the acceptance criteria; one commit per run. | `infra` (its `precheck` runs `infra`'s `run.sh check`) |
| [`infra`](../../plugins/infra.md) | Declares what the environment needs in `scripts/infra/requirements` and installs or checks it ([Declared environment](scripts.md#declared-environment)). | |
| [`qa`](../../plugins/qa.md) | Attacks the project and hands each real bug to `develop` as a failing test. | `develop` |
| [`router`](../../plugins/router.md) | The default router a `model` state with no `router:` asks ([The default router](runs.md#the-default-router)). | |
| [`newsletter`](../../plugins/newsletter.md) | Feeds in, picked by a local model, one markdown issue out, on a cron schedule. | |
| [`comfy`](../../plugins/comfy.md) | Renders images and videos on ComfyUI ([text-to-media](../../examples/text-to-media/README.md)). | |
| [`develop_by_size`](../../plugins/develop_by_size.md) | Sizes a change with a classifier, then implements it locally first or with Claude ([route-by-complexity](../../examples/route-by-complexity/README.md)). | `gliner_router` |
| [`gliner_router`](../../plugins/gliner_router.md) | A typed router: the GLiNER2.5-Decide classifier. | |
| [`llm_router`](../../plugins/llm_router.md) | A typed router: a local model held to the request's `reply_schema`. | |

Each one's body says, for someone new to decree, what it does, what it needs (services, tools, a cron file to write) and how to use it, and each one, added alone with its `needs` to an empty project, passes `decree check`. Every plugin carries every file its scripts use ([D77](../decisions.md#d77-a-machines-files-live-with-its-scripts)), so a file several use (`ai.sh`, `clean_tree.sh`, `commit.sh`, `keep.sh`) is a copy in each, kept identical by a test.

The `ai.sh` of `develop`, `infra`, `qa` and `develop_by_size`, and `router`'s `ask.sh`, pick the AI CLI when they run, so no file names a backend: the run's `ai_cli` data, else `AI_CLI`, else the first of `claude`, `opencode` and `copilot` on `PATH` ([The AI helper](scripts.md#the-ai-helper)).

**A new decree version.** The built-in copies change with decree. `decree plugin list` marks `differs` each plugin whose file in `plugins/` is not this decree's, and `decree plugin add --force <name>` takes this decree's copy, printing the project's edits it drops; put them back in the installed copy and run `decree plugin export <name>`.

## Converting an existing project

A project from before official plugins has its machines and scripts committed in `machines/` and `scripts/`. To commit only `plugins/`:

1. `decree plugin export <name>` for each machine, which writes `plugins/<name>.md` with the scripts it runs.
2. Add two lines to `.decree/.gitignore`: `machines/` and `scripts/`. decree never edits an existing `.gitignore`; `decree init` writes them in a new one.
3. `git rm -r --cached .decree/machines .decree/scripts`, then commit.

A fresh clone then has only `plugins/`, and `process`, `daemon`, `check` and `graph` install them before reading machines.

## Checks

`decree check` reports, each as an error with its rule ([Validation](machines.md#validation)):

- P1: a plugin file that fails the schema (a bad `name`, a `machine` with `name`, a script path such as `../x.sh`, an unknown key);
- P2: a `needs` that is not in `plugins/`;
- P3: a plugin that changed while its installed copy was edited in place.

It warns, without failing, about each machine that no plugin installed in a project that has `plugins/`, naming `decree plugin export <name>`.
