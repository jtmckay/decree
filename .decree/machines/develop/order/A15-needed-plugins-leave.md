---
machine: develop
---
# A15: An official plugin decree added as a need leaves with the last plugin that needed it

## Overview

`decree install <name>` copies into `plugins/` each official plugin that the named one extends or needs. For example, `decree install decree_develop_rust` prints `added .decree/plugins/decree_develop.md, needed by decree_develop_rust`, and adds `decree_infra`, `decree_order` and `decree_ai` the same way. Deleting `plugins/decree_develop_rust.md` later removes its installed copy, but every plugin it brought stays installed and keeps running. `decree_order` runs every machine's `order/` at each daemon start, so a project that turned a plugin off still runs code that only that plugin needed. Any project that turns plugins on and off has to track what each install added and remove it by hand.

Prior art: `apt-mark auto` and `apt autoremove`, `npm prune`, `pip-autoremove`. A package installed only as a dependency is marked as such, and it goes when nothing depends on it any more.

## Outcome

- Each plugin file that decree copies into `plugins/` because another plugin extends or needs it is recorded as **added for a need** in `.decree/cache/plugins/`. The record is only that mark. Which plugins need it is always worked out from the plugin files in `plugins/` as they are now, never stored, so it cannot go stale.
- A plugin is needed when a plugin in `plugins/` extends it, or lists it in `needs`. A `needs` map entry that meets the need with another plugin (`{decree_infra: infra}`) needs that other plugin, not the official one.
- `decree install` with no argument also removes the file and installed copy of each plugin added for a need when no plugin in `plugins/` needs it any more. It checks again after each removal, so a chain leaves together. It prints `removed <file>, no longer needed`, and archives an installed copy edited in place first, as every removal does (D96). The removed plugin's `state/` and `order/` stay, as they do today.
- `decree install <name>` of a plugin added for a need marks it as asked for. From then on it stays, as `apt install` of an automatic package does.
- A plugin file that a person wrote, or installed by name, is never removed this way. Neither is a plugin file whose record is missing, such as one from rc.1-rc.3, a fresh clone or a lost cache.
- `decree list` marks a plugin added for a need `(needed by <names>)`. `--format json` gives `needed_by`.

## Constraints

- No new commands, flags or plugin-file keys, and the plugin schema does not change.
- A project that never deletes a plugin behaves exactly as in rc.3. Every existing test passes unchanged.
- Tests use temp projects. No network and no real model.
- Docs:
  - `docs/reference/plugins.md`: the Installing section and the `install` rows of the command table.
  - the decree skill;
  - the CHANGELOG, under Added;
  - a decision in `docs/decisions.md` that cites the prior art above.
- Only this order's scope.
- If the reference docs and this order disagree in a way you cannot settle, write the question to a file named `STOP` in the run directory and end without further changes.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass, and `decree check` passes in every example directory.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply.

## Acceptance Criteria

- **Given** an empty project where `decree install decree_develop_rust` added `decree_develop`, `decree_infra`, `decree_order` and `decree_ai`
  **When** `plugins/decree_develop_rust.md` is deleted and `decree install` runs
  **Then** the four added plugin files and their `machines/<name>/` copies (except `state/` and `order/`) are gone, each printed as `removed <file>, no longer needed`

- **Given** that project, where `decree install decree_order` also ran before the delete
  **When** `decree install` runs
  **Then** `decree_order` stays and the other three go

- **Given** two plugins that both need `decree_ai`
  **When** one of them is deleted and `decree install` runs
  **Then** `decree_ai` stays, and `decree list` shows it `(needed by <the other>)`

- **Given** a plugin added for a need, and a project's own plugin that later starts needing it
  **When** the plugin that first needed it is deleted and `decree install` runs
  **Then** it stays, and `decree list` shows it `(needed by <the project's plugin>)`

- **Given** a `needs` map that meets `decree_infra` with the project's own `infra`, and a `decree_infra` added for a need
  **When** nothing else needs `decree_infra` and `decree install` runs
  **Then** `decree_infra` is removed and `infra` stays

- **Given** a `plugins/decree_ai.md` with no record in the cache
  **When** nothing needs it and `decree install` runs
  **Then** it stays

- **Given** a plugin added for a need, edited in place under `machines/`
  **When** the plugin that needed it is deleted and `decree install` runs
  **Then** the edited copy is archived in `.decree/archive/<name>/` before it is removed
