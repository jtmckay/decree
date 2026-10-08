# Editors

decree's machine schema ([Schema](reference/machines.md#schema)) lets an editor complete keys, show each key's description on hover, and underline a misspelled key or a wrong value as you type. Machines carry no line for it: editors find the schema by the file's path, through SchemaStore. Until then, or offline, a setting or a line in the file does the same.

The schemas live in [`schema/v1/`](../schema/v1/) in the decree repository, the same bytes decree is built from. Each one's `$id` is its URL there; the machine schema's is `https://raw.githubusercontent.com/jtmckay/decree/main/schema/v1/machine.schema.json`. That URL resolves once decree 0.5 is merged to `main`; while 0.5 is in beta, the same file is on the `v0.5` branch, `https://raw.githubusercontent.com/jtmckay/decree/v0.5/schema/v1/machine.schema.json`.

## SchemaStore

[SchemaStore](https://www.schemastore.org) is the catalog editors use to match files to schemas by path. Red Hat's YAML extension for VS Code, JetBrains IDEs and others read it by default, so a file that matches an entry is checked with no setting and no line in the file.

decree's entry, for [`src/api/json/catalog.json`](https://github.com/SchemaStore/schemastore/blob/master/src/api/json/catalog.json) in the SchemaStore repository:

```json
{
  "name": "decree machine",
  "description": "decree state machine: a statechart in .decree/machines/ that runs scripts, asks a model or a person, and starts child machines",
  "fileMatch": ["**/.decree/machines/*.yml", "**/.decree/machines/*.yaml"],
  "url": "https://raw.githubusercontent.com/jtmckay/decree/main/schema/v1/machine.schema.json"
}
```

The maintainer submits it once 0.5.0 is on `main`, so the `url` resolves when SchemaStore checks it. `docs_test.rs` holds the entry to the machine schema's `$id`.

## Until then, or offline

**VS Code.** With Red Hat's YAML extension, map the schema to the machines in the project's `.vscode/settings.json`. While 0.5 is in beta, use the `v0.5` branch's URL:

```json
{
  "yaml.schemas": {
    "https://raw.githubusercontent.com/jtmckay/decree/v0.5/schema/v1/machine.schema.json": [
      ".decree/machines/*.yml",
      ".decree/machines/*.yaml"
    ]
  }
}
```

Offline, run `decree schema` and map the local copy instead: `"./.decree/schema/v1/machine.schema.json"`. The copy is generated, and `.decree/.gitignore` lists `schema/`, so each person runs `decree schema` once, and again after an upgrade (`decree check` warns while the copy is out of date). Other editors that run the YAML language server take the same mapping in their own settings.

**Any editor.** The YAML language server also reads a comment at the top of the file. Put it above the machine's `# Graph:` line, after `decree schema` has written the local copy it points at:

```yaml
# yaml-language-server: $schema=../schema/v1/machine.schema.json
# Graph: ../graph/hello.md
```

The path is relative to the machine file. Both ways work, and they can be mixed: a machine with the line is checked against the local copy, one without it by the setting or SchemaStore.

## Messages and cron files

Migrations, inbox messages and cron files are Markdown with YAML frontmatter. Editors do not check frontmatter against a schema; `decree check` does, and any JSON Schema validator can check it against `message.schema.json`.
