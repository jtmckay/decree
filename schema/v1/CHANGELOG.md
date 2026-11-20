# Schema changelog (`v1`)

Changes to the schemas in this folder. Within `v1` changes are additive only ([Versioning](../../docs/reference/README.md#versioning)); the decree [changelog](../../CHANGELOG.md) lists every release.

## Unreleased

### Added

- `plugin.schema.json`: the frontmatter of a plugin, `.decree/plugins/<name>.md`: `name`, `machine` (without `name`), `scripts` (plain relative paths to their text) and `needs` ([plugins.md](../../docs/reference/plugins.md)).
- `cli/plugin.schema.json`: what `decree plugin list|add|export|import|install --format json` prints, one shape per `action`; `export` with no name prints the plugins it exported.
- `cli/check.schema.json`: `rule` may be `P1`, `P2` or `P3`, the plugin checks.
- `machine.schema.json`: a `reply` invoke's `attention` (default `true`); `false` marks a wait that needs no person.
- `events.schema.json`: a reply `waiting` event's `attention: false`, from that invoke.
- `cli/plugin.schema.json`: `export` with no name prints `conflicts`, the edited plugins it left to merge by hand.
- `plugin.schema.json`: `cron`, `migrations` and `store` (file names or plain paths to their text) and any `x-` key; `machine` is optional.
- `cli/plugin.schema.json`: `install` prints `removed` and `conflicts`, filled with no name.
- `cli/status.schema.json`: a `--cron` entry's `file` may be `<plugin>/<file>.md`, a cron file a plugin installed.
