# Plugins

A plugin is one machine's folder shared as one markdown file, `.decree/plugins/<name>.md`: the machine, the files of its scripts folder, its cron files, and seeds for its order files and its state. decree installs it into `.decree/machines/<name>/` (`machine.yml`, `scripts/`, `cron/`, `order/`, `state/`), where files written by hand would sit, so an installed machine runs exactly as if it had been written by hand: same paths, same `DECREE_*` variables, same [resolution](scripts.md#resolution). Every plugin is opt-in: nothing is installed until a project puts its file in `plugins/`, and nothing updates it but a change to that file. The plugin file is committed and its installed copy is not: `decree init`'s `.gitignore` ignores `machines/**` but each machine's `order/`, `graph.md` and `*.git.txt` files ([D86](../decisions.md#d86-machines-own-their-folders-order-replaces-migrations)). decree installs a plugin only when asked, by `decree plugin install`, `add` or `import`; every other command reads the installed copy, and `daemon` and `check` say which plugins are not installed as their files say. So in a fresh clone, or after a pull that changes a plugin, run `decree plugin install`; the official plugin [`plugin_install`](../../plugins/plugin_install.md) does it each time the daemon starts. Why: [D78](../decisions.md#d78-a-machine-is-shared-as-one-plugin-file), [D85](../decisions.md#d85-a-plugin-carries-any-of-decrees-files-and-installs-only-when-asked), [D86](../decisions.md#d86-machines-own-their-folders-order-replaces-migrations).

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
    echo "hi $(cat .decree/machines/hello/state/name.txt)"
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
| `name` | yes | Equal to the file's stem, and matches `^[a-z][a-z0-9_]*$`. Names the folder it installs into, `machines/<name>/`. |
| `machine` | no | The machine as `machines/<name>/machine.yml` holds it, without `name` ([machines.md](machines.md)). A plugin without one installs no machine: a plugin of scripts is a library (below). |
| `scripts` | no | A map of `<path>: <its text>`, each path relative to `machines/<name>/scripts/` and written as it is installed (`implement.sh`, `requirements`, `kinds/command.sh`). A path is plain and relative: no `..`, no `.`, no empty part, no leading `/`. |
| `cron` | no | A map of `<file>.md: <its text>`, each a [cron file](messages.md#cron-files) installed as `machines/<name>/cron/<file>.md`, whose `machine:` defaults to `<name>`. A file name, no `/`. |
| `order` | no | A map of `<file>.md: <its text>`, each an [order file](messages.md#order-files-run-in-order-at-each-start) written to `machines/<name>/order/<file>.md` once. A file name, no `/`; it sorts among the project's order files by name, so prefix it as the project does (`A01-newsletter-setup.md`). |
| `state` | no | A map of `<path>: <its text>`, seeds for `machines/<name>/state/` (`$DECREE_STATE_DIR`), each written once. A plain relative path, as in `scripts`. |
| `needs` | no | The other plugins it relies on: a machine it invokes, emits to or routes through (`router:`, or the machine `router` that a `model` state with no `router:` uses), or a library whose scripts it sources. |
| `x-<anything>` | no | Ignored by decree and kept as written by export: room for a tool built on decree (existential's `x-exist-services`), as Compose and OpenAPI keep `x-` keys. |

No other key is allowed. An installed script is executable when it starts with `#!`, and otherwise not. `.decree/schema/v1/plugin.schema.json` describes the frontmatter ([Schemas](README.md#schemas)).

**What each key's files become.** `machine`, `scripts` and `cron` are the plugin's installed copy: installing makes them exactly what the plugin says, an edit to them in place is an edit (below), and they go when the plugin goes. `order` and `state` are seeds, written once, never overwritten:

- An order file or state file is written only when it is absent. After that it is the project's: an order file is committed with its history as any other, and the state folder is the project's data. A plugin never overwrites or removes either, even when the plugin goes.

**Libraries.** A plugin with no `machine` installs only `machines/<name>/scripts/` (and any cron files and seeds). Another plugin that uses its scripts lists it in `needs`, so `decree check` (P2) and `decree plugin import` refuse to go without it, and sources it by its installed path from the project root, every script's working directory: `source .decree/machines/<name>/scripts/<file>`. The official `order` plugin is one: its `emit_if_changed.sh` and `emit_if_changed_git.sh` are for any machine. Coupling plugins is explicit: the `needs` entry and the path, nothing implicit. A library is changed like any plugin, by changing its file, and every plugin that sources it gets the change.

## Installing

Installing is a step a person (or a machine of theirs) takes, never a side effect: a pull or an edit to a plugin file changes nothing that runs until it is installed. `decree plugin install` with no name installs every plugin in `plugins/` that is new or has changed since it was installed, removes the installed copy of each deleted one, and keeps every copy edited in place (below), exiting 1 if a plugin is a conflict or fails its schema. `decree plugin install <name>...` installs those now, over any edit. `add` and `import` install what they write.

`daemon` and `check` install nothing. They compare `plugins/` with what is installed and say what `decree plugin install` would do: `daemon` print each such plugin once, on stderr (``plugins/<name>.md: not installed, or changed since it was installed: run `decree plugin install` ``), and look again before each run, as they reload machines ([Edits while running](cli.md#edits-while-running)); `check` warns about it and reports P1–P3 as errors. A project whose `machines/` is read-only (a container mounting them from the host) runs as installed, and the host installs. To install on every daemon start, add the official plugin `plugin_install`. A project without `plugins/` works as before.

- A plugin's installed copy is exactly `machines/<name>/machine.yml` and the files under `machines/<name>/scripts/` and `machines/<name>/cron/`, so two plugins never write the same file. A file in `scripts/` or `cron/` that the plugin does not ship is removed when it is installed: the folders always match the plugin.
- Its order and state seeds are written once, as above.
- When a plugin file is deleted, its installed copy is removed; its `state/`, `order/` and `graph.md` stay.
- A machine folder that no plugin installed is never touched.
- What decree records about an install is a copy of the plugin file as installed, `.decree/cache/plugins/<name>.md`, under the gitignored `cache/`.

**Edits in place.** Nothing is overwritten when a plugin's installed copy was edited in place. Adding, changing or deleting a file in `machines/<name>/scripts/` or `cron/`, or editing `machine.yml`, all count as edits; the machine is compared as YAML, so a comment or the order of its keys is not one. Deleting the whole installed copy (machine and folders) is not an edit: it is installed again, as in a fresh clone.

- If the plugin file has not changed, the edit stays and runs.
- If the plugin file changed too (after a `git pull`, say), `decree plugin install` does not install it again: the edited copy keeps running, `install` and `check` exit 1, `daemon` print the conflict once, and `decree check` reports it with a P3 error naming the plugin and both ways out: `decree plugin export <name>` keeps the edit (merge the plugin file with git), and `decree plugin install <name>` takes the plugin's version.
- An installed copy that exists but was never installed by decree (a project converting to plugins) is treated the same way: if it matches the plugin, it is recorded; if it differs, it is a conflict.
- A deleted plugin whose installed copy was edited keeps that copy, which is then a machine no plugin installed.

So the plugin is the source: edit the installed copy, try it, then run `decree plugin export <name>` and commit the plugin file. A project's own changes to a plugin (a `gate.sh`, `requirements`, infra's kinds, qa's invariants, wrenches and `QA_RUN`) live in its plugin file, not in a separate configuration folder ([D79](../decisions.md#d79-official-plugins-are-built-in-and-opt-in)), so sharing the plugin shares them too.

**A run's edits.** The `commit` script the official plugins carry (`done`'s `onentry`) runs `decree plugin export` with no name before it commits, so what a run changed in an installed copy (an `infra` run adding a line to `machines/infra/scripts/requirements`, a `develop` run editing its gate) reaches `plugins/` and is in the run's one commit; git ignores the installed copy. A run that fails exports nothing: `keep` leaves its edits in the installed copy, where `decree daemon --once --retry` builds on them, and counts them (`decree plugin export --dry-run`) among the changes that make its attention file hold the queue, so the next run's `commit` does not take them.

## Commands

| Command | Behaviour |
| --- | --- |
| `decree plugin list [--format <text\|json>]` | Lists the [official plugins](#official-plugins) built into this decree, one line each with its machine's `description`, marked `added` when `plugins/` has a plugin of that name and `differs` when that file is not this decree's copy byte for byte (the project edited it, or another decree version wrote it). |
| `decree plugin add <name>... [--force] [--format <text\|json>]` | Copies the named official plugins into `plugins/`, with every official plugin they need (their `needs`, and theirs) that `plugins/` does not have, installs them and redraws the graphs. It prints `added <file>` for each, and `added <file>, needed by <names>` for each it added because another needs it. A named plugin already in `plugins/` is left as it is: `kept <file>: … nothing changed`. With `--force` it is replaced by this decree's copy, and the output lists each file that changes with the lines dropped (`-`) and added (`+`), so the project's own edits it drops are on screen, and says if the installed copy had edits in place that were not exported. A name that is not an official plugin exits 1, writing nothing, and lists the official names. A need that `plugins/` already has is the project's own and is left alone; a `machines/<name>/machine.yml` that no plugin installed exits 1, naming `decree plugin export <name>`. |
| `decree plugin export [<name>] [--dry-run] [--format <text\|json>]` | Writes `plugins/<name>.md` from the installed copy: the machine, if there is one; every file in `machines/<name>/scripts/` and `cron/`; and `needs`, every other machine it invokes, emits to or routes through, plus the `needs` the existing plugin file has (a plugin may rely on another in a way its machine does not show: `develop`'s `precheck` runs `infra`'s `run.sh`). An existing plugin file keeps its body, its `order`, its `state` and its `x-` keys, which are not read back from `order/` and `state/`; a new one gets `# <name>` and the machine's `description`. Works on a machine that was never a plugin, which is how a project converts. Prints the file and what it installs. Without a name it exports every plugin whose installed copy differs from what its file installs (edited in place, by a person or a run), and prints `wrote <file>` for each; `--dry-run` prints `would write <file>` and writes nothing. A conflict, an edited copy whose plugin file changed since it was installed (a pull), is never exported this way, since that would write the old version over the new file: it prints `not exported: <file> ...` and is left to merge by hand. |
| `decree plugin import <file\|-> [--force] [--format <text\|json>]` | Checks a plugin file (`-` reads stdin), copies it to `plugins/<name>.md` and installs it. Before installing it prints what the plugin runs: the machine's description, each file with its line count, and `needs`. It refuses, exit 1 and writing nothing, a file that fails the schema; a plugin whose `needs` are not in `plugins/`, naming them; and a name that `plugins/` (or a hand-written `machines/<name>/machine.yml`) already has, unless `--force`, with which it prints each file added, changed or removed and installs it over any edit. decree fetches nothing: `curl -fsSL <url> \| decree plugin import -` imports from a URL. |
| `decree plugin install [<name>...] [--format <text\|json>]` | With no name, installs every plugin that is new or changed, removes the installed copy of each deleted plugin and keeps every copy edited in place ([Installing](#installing)), printing `installed <file>` and `removed the installed copy of <file>, which was deleted` for each; a conflict or a plugin that fails the schema is named on stderr and the exit code is 1. With names, installs those now, replacing an installed copy that was edited in place: this is how an edit is discarded; exit 1, installing nothing, if a name has no plugin file or one fails the schema. |

Export is deterministic, so git diffs show only real changes: installing a plugin and exporting it gives the same bytes, and exporting the same installed copy twice gives the same file. Keys come in the order `name`, `needs`, `machine`, `scripts`, `cron`, `state`, `order`, then the `x-` keys as written; the machine's keys as its file has them, and each map's files by path. `--format json` prints one document, described by `.decree/schema/v1/cli/plugin.schema.json`.

## Official plugins

decree is built with its official plugins: the decree repository's [`plugins/`](../../plugins/), each compiled into the binary, so adding one needs no network and its version is the binary's. `decree init` adds `order` and `prune`, which a project drops by deleting their files; the others are opt-in: `decree plugin list` shows them and `decree plugin add <name>` adds one ([D86](../decisions.md#d86-machines-own-their-folders-order-replaces-migrations)).

| Plugin | What it does | Needs |
| --- | --- | --- |
| [`develop`](../../plugins/develop.md) | Implements a message with an AI agent, runs the project's gate, has the agent fix a failing gate, then check the acceptance criteria; one commit per run. | `infra` (its `precheck` runs `infra`'s `run.sh check`) |
| [`develop_rust`](../../plugins/develop_rust.md) | `develop` for a Rust project: its gate runs `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test`, and its prompts follow Rust conventions ([D84](../decisions.md#d84-develop-variants-are-separate-machines)). | `infra` |
| [`develop_node`](../../plugins/develop_node.md) | `develop` for a Node project: its gate installs with the lockfile's package manager (npm, pnpm or yarn) as CI does, then runs `lint`, `typecheck` and `test` where `package.json` defines them. | `infra` |
| [`develop_react`](../../plugins/develop_react.md) | `develop` for a React project: `develop_node`'s gate, then `build`; its prompts ask for component tests with Testing Library. | `infra` |
| [`infra`](../../plugins/infra.md) | Declares what the environment needs in `machines/infra/scripts/requirements` and installs or checks it ([Declared environment](scripts.md#declared-environment)). | |
| [`qa`](../../plugins/qa.md) | Attacks the project and hands each real bug to `develop` as a failing test. | `develop` |
| [`router`](../../plugins/router.md) | The default router a `model` state with no `router:` asks ([The default router](runs.md#the-default-router)). | |
| [`newsletter`](../../plugins/newsletter.md) | Feeds in, picked by a local model, one markdown issue out, on a cron schedule. | |
| [`comfy`](../../plugins/comfy.md) | Renders images and videos on ComfyUI ([text-to-media](../../examples/text-to-media/README.md)). | |
| [`develop_by_size`](../../plugins/develop_by_size.md) | Sizes a change with a classifier, then implements it locally first or with Claude ([route-by-complexity](../../examples/route-by-complexity/README.md)). | `gliner_router` |
| [`gliner_router`](../../plugins/gliner_router.md) | A typed router: the GLiNER2.5-Decide classifier. | |
| [`llm_router`](../../plugins/llm_router.md) | A typed router: a local model held to the request's `reply_schema`. | |
| [`order`](../../plugins/order.md) | Runs each machine's `order/` files at each daemon start, in file-name order, each once and again when it changes, recording each in that machine's `state/order.git.txt` ([Order files](messages.md#order-files-run-in-order-at-each-start)). Also a library: `emit_if_changed.sh` and `emit_if_changed_git.sh` queue a message only when it differs from the last one. Added by `decree init`. | |
| [`prune`](../../plugins/prune.md) | Each week, deletes the folders of finished runs older than `PRUNE_OLDER_THAN` (`30d`), but a child whose parent is not finished and a run a process holds. Added by `decree init`. | |
| [`plugin_install`](../../plugins/plugin_install.md) | Runs `decree plugin install` each time `decree daemon` starts (its cron file is `@reboot`), so a restart after a pull runs the plugins as pulled. | |
| [`lease`](../../plugins/lease.md) | One holder per resource (a GPU, an LLM server, a paid API) for a limited time, the next picked to keep mode swaps rare ([D83](../decisions.md#d83-leases-are-an-official-plugin-not-core)). | |

Each one's body says, for someone new to decree, what it does, what it needs (services, tools, a cron file to write) and how to use it, and each one, added alone with its `needs` to an empty project, passes `decree check`. Every plugin carries every file its scripts use ([D77](../decisions.md#d77-a-machines-files-live-with-its-scripts)), so a file several use (`ai.sh`, `clean_tree.sh`, `commit.sh`, `keep.sh`) is a copy in each, kept identical by a test.

The `develop` variants (`develop_rust`, `develop_node`, `develop_react`) are `develop`'s machine under their own names, so one project can add two and point each message at one. Each carries `develop`'s scripts but for its gate, the `conventions.md` its implement, fix and verify prompts follow, and the tools it declares in `machines/<variant>/scripts/requirements`, which `precheck` checks; a test keeps the rest identical to `develop`'s ([D84](../decisions.md#d84-develop-variants-are-separate-machines)). A project in another ecosystem starts from `develop` and fills in its gate.

The `ai.sh` of `develop` and its variants, `infra`, `qa` and `develop_by_size`, and `router`'s `ask.sh`, pick the AI CLI when they run, so no file names a backend: the run's `ai_cli` data, else `AI_CLI`, else the first of `claude`, `opencode` and `copilot` on `PATH` ([The AI helper](scripts.md#the-ai-helper)).

**A new decree version.** The built-in copies change with decree. `decree plugin list` marks `differs` each plugin whose file in `plugins/` is not this decree's, and `decree plugin add --force <name>` takes this decree's copy, printing the project's edits it drops; put them back in the installed copy and run `decree plugin export <name>`.

## Converting an existing project

A project from before 0.6 has `machines/<name>.yml`, `scripts/<name>/`, `store/<name>/`, `cron/`, `migrations/` and `processed.md`. To convert ([D86](../decisions.md#d86-machines-own-their-folders-order-replaces-migrations); the CHANGELOG lists each step):

1. Move each `machines/<name>.yml` to `machines/<name>/machine.yml`, `scripts/<name>/` to `machines/<name>/scripts/` (a flat `scripts/<file>` into each machine that runs it), `store/<name>/` to `machines/<name>/state/` (and `store:` to `state_files:`), and each cron file to `machines/<machine>/cron/`.
2. Move `migrations/*.md` into the `order/` of the machine each names, add the `order` plugin (`decree plugin add order`), and write each machine's `state/order.git.txt`, one `<file> <sha256>` line per file `processed.md` lists; then delete `processed.md`.
3. `decree plugin export <name>` for each machine, which writes `plugins/<name>.md`.
4. Replace `.decree/.gitignore` with the one `decree init` writes (decree never edits an existing one), `git rm -r --cached .decree/machines`, then `git add .decree` and commit.

A fresh clone then has only `plugins/` and the committed order files, records and graphs: run `decree plugin install` once, and again after a pull that changes a plugin (or add `plugin_install`).

## Checks

`decree check` reports, each as an error with its rule ([Validation](machines.md#validation)):

- P1: a plugin file that fails the schema (a bad `name`, a `machine` with `name`, a script path such as `../x.sh`, a cron file or order file name with `/` or without `.md`, an unknown key that does not start with `x-`);
- P2: a `needs` that is not in `plugins/`;
- P3: a plugin that changed while its installed copy was edited in place.

It warns, without failing, about each machine that no plugin installed in a project that has `plugins/`, naming `decree plugin export <name>`.
