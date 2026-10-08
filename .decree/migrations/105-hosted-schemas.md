---
machine: develop
---
# 105: Schemas hosted on GitHub and registered with SchemaStore; `.decree/schema/` becomes optional

## Overview

decree validates with schemas built into the binary; `.decree/schema/` exists for editors (each machine's `# yaml-language-server: $schema=../schema/v1/machine.schema.json` line) and for AI agents (the skill says to read the machine schema). The user chose: publish the schemas at a stable URL, register them with SchemaStore so editors apply them with no line in the file, stop `decree init` from writing `.decree/schema/`, and keep `decree schema` for agents and offline use.

SchemaStore (https://www.schemastore.org) is the catalog editors use to match files to schemas by path: Red Hat's YAML extension for VS Code, JetBrains IDEs and others read it by default.

## Requirements

1. **One copy, at a stable path.** Move `src/templates/schema/` to `schema/` at the repository root (`git mv`), and embed it from there (`include_str!`), so the published files and the binary's are the same bytes.
   - Each schema's `$id` is `https://raw.githubusercontent.com/jtmckay/decree/main/schema/v1/<path>` (for example `…/schema/v1/machine.schema.json`, `…/schema/v1/cli/check.schema.json`). It resolves once 0.5 is merged to `main`; say so where it matters (requirement 4).
   - References between schemas, if any, keep working (relative to the `$id`).
2. **`decree init`** no longer writes `.decree/schema/`, and the machines it writes no longer start with a `# yaml-language-server: $schema=…` line (they keep `# Graph: …`). Its `.decree/.gitignore` adds `schema/`: a local copy is generated, not committed.
3. **`decree schema`** is unchanged in what it writes (`.decree/schema/v1/`), and now optional. `decree check` warns that a schema file is out of date only when `.decree/schema/` exists; with no folder it says nothing. A machine that still has a `$schema=../schema/…` line keeps working when the folder exists, and the docs say both ways work.
4. **Editors.** A new `docs/editors.md`:
   - **SchemaStore:** the catalog entry to submit (name `decree machine`, description, `fileMatch` `["**/.decree/machines/*.yml", "**/.decree/machines/*.yaml"]`, `url` the machine schema's `$id`), and a note that the user submits it once 0.5.0 is on `main`.
   - **Until then, or offline:** a VS Code `.vscode/settings.json` `yaml.schemas` mapping from the machine schema's URL (the `v0.5` branch's raw URL while 0.5 is in beta) to `.decree/machines/*.yml`, or a local path after `decree schema`; and the per-file `$schema` line as the editor-independent fallback.
   - Messages and cron files are markdown: editors do not check their frontmatter; `decree check` does.
5. **Agents.** The skill: "Before writing a machine, run `decree schema` and read `.decree/schema/v1/machine.schema.json`" (the folder is generated and ignored). Same for the router and `events.jsonl` schemas.
6. **This repository and the examples:** remove the `$schema` lines from every machine (templates, examples, this repository's `.decree/machines/`, test fixtures where the line is not what the test is about), remove the committed `.decree/schema/` folders from the examples and from this repository's `.decree/`, and add `schema/` to their `.decree/.gitignore`. Docs that quote the line or the folder are updated (`docs/reference/README.md` layout, `machines.md` "Schema", README, `help.txt`, the skill, `cli.md`'s `schema` and `init` rows).
7. **Tests:** the embedded schemas are byte-identical to `schema/v1/`; every `$id` has the URL form above; `init` writes neither the folder nor the line; `decree check` with no `.decree/schema/` gives no schema warning, and with a stale folder still warns; `decree schema` still writes every file; the SchemaStore entry in `docs/editors.md` is valid JSON whose `url` is the machine schema's `$id`.
8. CHANGELOG under Changed, and a decision in `docs/decisions.md` (hosted schemas, SchemaStore, local copy optional; why).

- Only this migration's scope.
- Never edit `.decree/migrations/`, `.decree/runs/` or `.decree/processed.md`, not even by a search and replace across the repository: exclude them from every bulk edit.
- If the reference docs and this migration disagree in a way you cannot settle, write the question to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- No test calls a real model or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass, and `decree check` passes in every example directory.

## Acceptance Criteria

- **Given** `decree init` in an empty directory
  **When** it finishes
  **Then** there is no `.decree/schema/`, no machine has a `$schema` line, `.decree/.gitignore` lists `schema/`, and `decree check` passes with no warning

- **Given** the repository
  **When** `schema/v1/machine.schema.json` is read
  **Then** its `$id` is `https://raw.githubusercontent.com/jtmckay/decree/main/schema/v1/machine.schema.json`, and it is byte-identical to what `decree schema` writes

- **Given** the repository
  **When** `rg -n --hidden 'yaml-language-server' --glob '!.decree/migrations/**' --glob '!.decree/runs/**' --glob '!.git/**' --glob '!CHANGELOG.md'` runs
  **Then** it matches only `docs/editors.md` and the docs that explain the fallback line
