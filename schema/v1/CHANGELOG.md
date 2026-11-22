# Schema changelog (`v1`)

Changes to the schemas in this folder. Within `v1` changes are additive only, but decree 0.x may still change `v1`, listing each change here ([Versioning](../../docs/reference/README.md#versioning)); the decree [changelog](../../CHANGELOG.md) lists every release.

## Unreleased

### Added

- `machine.schema.json`: a `machine` invoke may name a `message` file in place of `name`; `state_files` (was `store`).
- `machine.schema.json`: root `concurrency` (default 1) and `concurrency_group`; `cli/status.schema.json`: `executing`, each machine's runs executing now and its limit.
- `machine.schema.json`: a `reply` invoke may take `timeout_at`, in place of `timeout`: `<file>` or `<file>#<JSON Pointer>` in the run's folder, the instant the wait ends.
- `cli/daemon.schema.json`: what `decree daemon --once --dry-run --format json` prints (was `cli/process.schema.json`).
- `cli/reply.schema.json`, what `decree reply --format json` prints (was `cli/event.schema.json`, with `decree event`).
- `cli/plugin.schema.json`: `list` is `decree list`'s shape (`plugins` with `official`, `differs` and `state`; `unplugged`; `available`).
- `cli/plugin.schema.json`: also describes `decree install`; the named form's `action` is `install` (was `add`, whose `kept` status is `installed`), and the no-argument form's is `sync` (was `install`).
- `message.schema.json`: a message other than a reply may leave out `machine`, as a cron file or order file does; `decree check` (M2) still requires it in `inbox/`.
- `plugin.schema.json`: the frontmatter of a plugin, `.decree/plugins/<name>.md`: `name`, `machine` (without `name`; optional), `scripts` and `cron` (plain relative paths to their text), `state` and `order` (seeds, written once), `needs` and any `x-` key ([plugins.md](../../docs/reference/plugins.md)).
- `cli/plugin.schema.json`: what `decree install`, `decree list` and `decree export --format json` print, one shape per `action`; `export` with no name prints the plugins it exported and `conflicts`, the edited plugins it left to merge by hand; `sync` prints `installed`, `removed` and `conflicts`; `import` prints `added`, the official plugins it seeded.
- `cli/check.schema.json`: `rule` may be `P1`, `P2` or `P3`, the plugin checks.
- `machine.schema.json`: a `reply` invoke's `attention` (default `true`); `false` marks a wait that needs no person. `events.schema.json`: a reply `waiting` event's `attention: false`, from that invoke.
- `cli/status.schema.json`: a `--cron` entry's `file` may be `<plugin>/<file>.md`, a cron file a plugin installed; `executing` gives a machine in a group its `group` and `group_running`.

### Removed

- `cli/prune.schema.json`, with `decree prune`, and `cli/skill.schema.json`, with `decree skill`.
- `message.schema.json`: the `migration` key and the `migration` trigger; `cli/status.schema.json` and `cli/check.schema.json`: their migration fields (`status`'s `blocked` and `queued.migrations`, `check`'s rule `M1`); `attention.schema.json`'s `message` is always `inbox`.

