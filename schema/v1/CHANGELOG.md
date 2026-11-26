# Schema changelog (`v1`)

Changes to the schemas in this folder. Within `v1` changes are additive only, but decree 0.x may still change `v1`, listing each change here ([Versioning](../../docs/reference/README.md#versioning)); the decree [changelog](../../CHANGELOG.md) lists every release.

## Unreleased

### Added

- `events.schema.json`: a child `waiting` event's `message`, the file a `machine: { message }` invoke runs; such a child is its own root run.
- `plugin.schema.json`: `extends` (the plugin this one is installed on top of), `source` (the https URL it is published at), a `needs` entry may be a one-key map `{<need>: <plugin>}`, and a `scripts`, `cron`, `state` or `order` path may be `null` in a plugin that extends another. A `name` starting with `decree_` is an official plugin's.
- `cli/plugin.schema.json`: `sync`'s `restored`: each official plugin whose file or installed copy was not this decree's, archived and replaced with it, with its changes. `list`'s plugins give `extends`, and `state` may be `missing base`. `install`'s status may be `reinstalled`. `export` with no name gives `official`, the official plugins whose installed copy was edited, never exported. The `update` action, what `decree install --update --format json` prints.
- `cli/check.schema.json`: the rule `P3`, an official plugin that is not this decree's copy.
- `cli/plugin.schema.json`: `archived`, the plugin file (or, for a copy that cannot be one, the folder) an install archived what it replaced as: `sync` lists `{ "plugin", "archive" }` in place of `conflicts`; `install` gives each plugin's folder or `null` in place of `dropped_edits_in_place`; `import` gives its folder or `null`. `list`'s `state` loses `conflict`.
- `cli/status.schema.json`: `queued.held`, the inbox files a hold keeps queued, and a run's `held: true` for a `waiting` or `pending` run of a held machine.
- `cli/cancel.schema.json`: what `decree cancel --format json` prints, `{ "cancelled": [id] }`. `events.schema.json`: the `cancel` source of a `transition` (with its `note`), and `run_finished`'s `cancelled`. `cli/status.schema.json`: cancelled runs are under `finished.cancelled`.
- `cli/retry.schema.json`: what `decree retry --format json` prints, `{ "id", "state" }`.
- `machine.schema.json`: a `machine` invoke may name a `message` file in place of `name`; `state_files` (was `store`).
- `machine.schema.json`: root `concurrency` (default 1) and `concurrency_group`; `cli/status.schema.json`: `executing`, each machine's runs executing now and its limit.
- `machine.schema.json`: a `reply` invoke may take `timeout_at`, in place of `timeout`: `<file>` or `<file>#<JSON Pointer>` in the run's folder, the instant the wait ends.
- `cli/reply.schema.json`, what `decree reply --format json` prints (was `cli/event.schema.json`, with `decree event`).
- `cli/plugin.schema.json`: `list` is `decree list`'s shape (`plugins` with `official`, `differs` and `state`; `unplugged`; `available`).
- `cli/plugin.schema.json`: also describes `decree install`; the named form's `action` is `install` (was `add`, whose `kept` status is `installed`), and the no-argument form's is `sync` (was `install`).
- `message.schema.json`: a message other than a reply may leave out `machine`, as a cron file or order file does; `decree check` (M2) still requires it in `inbox/`.
- `plugin.schema.json`: the frontmatter of a plugin, `.decree/plugins/<name>.md`: `name`, `machine` (without `name`; optional), `scripts` and `cron` (plain relative paths to their text), `state` and `order` (seeds, written once), `needs` and any `x-` key ([plugins.md](../../docs/reference/plugins.md)).
- `cli/plugin.schema.json`: what `decree install`, `decree list` and `decree export --format json` print, one shape per `action`; `export` with no name prints the plugins it exported and `conflicts`, the edited plugins it left to merge by hand; `sync` prints `installed`, `removed` and `archived`; `import` prints `added`, the official plugins it seeded.
- `cli/check.schema.json`: `rule` may be `P1` or `P2`, the plugin checks.
- `machine.schema.json`: a `reply` invoke's `attention` (default `true`); `false` marks a wait that needs no person. `events.schema.json`: a reply `waiting` event's `attention: false`, from that invoke.
- `cli/status.schema.json`: a `--cron` entry's `file` may be `<plugin>/<file>.md`, a cron file a plugin installed; `executing` gives a machine in a group its `group` and `group_running`.

### Removed

- `cli/schema.schema.json`, with `decree schema`, and `cli/process.schema.json`, with `decree process --dry-run`.
- `cli/stop.schema.json`, with `decree stop`: `cli/pause.schema.json` describes `decree pause` and `decree resume`. `attention.schema.json`'s `manual-stop` kind is `pause` (`.decree/attention/pause.md`), and so in `cli/status.schema.json`.
- `cli/prune.schema.json`, with `decree prune`, and `cli/skill.schema.json`, with `decree skill`.
- `message.schema.json`: the `migration` key and the `migration` trigger; `cli/status.schema.json` and `cli/check.schema.json`: their migration fields (`status`'s `blocked` and `queued.migrations`, `check`'s rule `M1`); `attention.schema.json`'s `message` is always `inbox`.

