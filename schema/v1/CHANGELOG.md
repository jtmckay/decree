# Schema changelog (`v1`)

Changes to the schemas in this folder. Within `v1` changes are additive only ([Versioning](../../docs/reference/README.md#versioning)); the decree [changelog](../../CHANGELOG.md) lists every release.

## Unreleased

### Added

- `machine.schema.json`: a `machine` invoke may name a `message` file in place of `name`; `state_files` (was `store`).
- `cli/daemon.schema.json`: what `decree daemon --once --dry-run --format json` prints (was `cli/process.schema.json`).
- `plugin.schema.json`: `state` and `order` (were `store` and `migrations`).
- `message.schema.json`: a message other than a reply may leave out `machine`, as a cron file or order file does; `decree check` (M2) still requires it in `inbox/`.

### Removed

- `cli/prune.schema.json`, with `decree prune`, and `cli/skill.schema.json`, with `decree skill`.
- `message.schema.json`: the `migration` key and the `migration` trigger; `cli/status.schema.json` and `cli/check.schema.json`: their migration fields (`status`'s `blocked` and `queued.migrations`, `check`'s rule `M1`); `attention.schema.json`'s `message` is always `inbox`.

- `plugin.schema.json`: the frontmatter of a plugin, `.decree/plugins/<name>.md`: `name`, `machine` (without `name`), `scripts` (plain relative paths to their text) and `needs` ([plugins.md](../../docs/reference/plugins.md)).
- `cli/plugin.schema.json`: what `decree plugin list|add|export|import|install --format json` prints, one shape per `action`; `export` with no name prints the plugins it exported.
- `cli/check.schema.json`: `rule` may be `P1`, `P2` or `P3`, the plugin checks.
- `machine.schema.json`: a `reply` invoke's `attention` (default `true`); `false` marks a wait that needs no person.
- `events.schema.json`: a reply `waiting` event's `attention: false`, from that invoke.
- `cli/plugin.schema.json`: `export` with no name prints `conflicts`, the edited plugins it left to merge by hand.
- `plugin.schema.json`: `cron`, `migrations` and `store` (file names or plain paths to their text) and any `x-` key; `machine` is optional.
- `cli/plugin.schema.json`: `install` prints `removed` and `conflicts`, filled with no name.
- `cli/status.schema.json`: a `--cron` entry's `file` may be `<plugin>/<file>.md`, a cron file a plugin installed.
