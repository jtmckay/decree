---
machine: rust_develop
---
# 72: JSON Schema for machines and messages

## Overview

The machine format is described only in prose (`docs/reference/machines.md`). A JSON Schema makes it typesafe where people and models write it:

- **Editors:** the YAML language server (VS Code's Red Hat YAML extension and others) completes keys and underlines mistakes as you type, from a `# yaml-language-server: $schema=…` line.
- **Models:** a model reads one precise contract, and can be held to it when generating a machine.
- **Tools:** any JSON Schema validator can check a machine without decree.

JSON Schema (draft 2020-12) is the standard for this, and SchemaStore and the YAML language server use it. The schema covers what a schema can express: keys, types, required fields, the shape of each `invoke` kind and each condition, the name patterns, and the ranges. `decree check` remains the authority for the rest: reachability, targets that exist, scripts that resolve, cycles, and the other cross-references in V1–V21. The schema never accepts a machine that `decree check` would reject for its shape.

This follows migration 71, so the schema describes the new shapes only.

## Requirements

Read `docs/reference/machines.md`, `docs/reference/messages.md`, `docs/reference/cli.md` and `tests/README.md` first, then `src/machine.rs`, `src/machine/validate.rs` and `src/commands/graph.rs`, which writes `.decree/graph/` the way this migration writes `.decree/schema/`.

1. **Write two schemas**, draft 2020-12, as the single source in `src/templates/schema/`, compiled into the binary:
   - **`machine.schema.json`:** the root, states (atomic, compound and final, as `oneOf` or `if`/`then` so each kind allows only its keys), the five `invoke` kinds and their short forms, transitions (short and long form, including `true`/`false` keys), conditions (exactly one subject and one operator, `<value>` as a literal or `{ data: <name> }`), `data`, and every name pattern from the Rules. Use `additionalProperties: false` everywhere, as V19 does. Every property has a `description` taken from the reference docs, so editors show it on hover and a model reading the schema learns what each key means.
   - **`message.schema.json`:** the frontmatter of a message. It has two shapes: a message (`machine` or `routine`, with `params` and the other keys in `docs/reference/messages.md`, any other key allowed) and a reply (`to` and `event`).
   - Give each schema an `$id` and a `title`.
2. **`decree schema`** writes both files to `.decree/schema/`, through a temp file and a rename, and prints what it wrote. `decree init` writes them too. `decree check` warns, without failing, when `.decree/schema/` is missing or differs from what `decree schema` would write, as it does for `.decree/graph/`.
3. **Machine files point at their schema.** Every machine `decree init` writes, and every machine in `mock/` and `examples/`, starts with `# yaml-language-server: $schema=../schema/machine.schema.json`, above its existing `# Graph:` line. Run `decree schema` in `mock/`, in each project in `examples/` and in this repository, and commit the files. Keep `tests/mock_templates_test.rs` pairing the mock's copies with the templates.
4. **Tests**, adding `jsonschema` as a dev-dependency only (a schema validator; not linked into the binary):
   - every machine in `mock/`, `examples/`, `src/templates/` and a fresh `decree init` for each `--ai` validates against `machine.schema.json`;
   - every message, migration and cron file in `mock/` validates against `message.schema.json`;
   - for every failing case in `tests/validation_test.rs` that is about shape (V19, the shape parts of V10, V16 and V18, and each old shape from migration 71), the schema rejects the machine too. List the failing cases the schema cannot express, and why: these stay with `decree check`;
   - `decree schema` writes both files, and `decree check` warns when they are missing or stale;
   - both schemas are valid draft 2020-12 schemas.
5. **Docs:**
   - `docs/reference/machines.md`: a short section "Schema" (what it checks, what only `decree check` checks, the `$schema` line);
   - `docs/reference/cli.md`: a row for `decree schema`, and the `init` and `check` rows updated;
   - `docs/reference/README.md`: the `.decree/schema/` folder in the layout;
   - the decree skill (`src/templates/skills/decree/`): tell a model to read `.decree/schema/machine.schema.json` before writing a machine;
   - `src/templates/help.txt`, `README.md` and `tests/readme_test.rs` (the command list);
   - `docs/decisions.md`: an entry for "JSON Schema for shape, `decree check` for meaning".

- Only this migration's scope.
- If the reference docs and the code disagree, or a case is not covered here, do not guess: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- Tests build their own `.decree/` in a temp directory and never touch this repository's `.decree/`, except the explicit `decree schema` run in requirement 3. No test calls a real LLM or the network. The only new dependency is `jsonschema`, as a dev-dependency.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass.

## Files to Modify

- src/templates/schema/ (new), src/cli.rs, src/commands/schema.rs (new), src/commands/init.rs, src/commands/check.rs
- every machine in mock/, examples/, src/templates/ and .decree/machines/; every .decree/schema/
- docs/reference/, docs/decisions.md, README.md, src/templates/help.txt, src/templates/skills/decree/
- Cargo.toml (dev-dependency), tests/

## Acceptance Criteria

- **Given** every machine in `mock/`, `examples/`, the templates and a fresh `decree init`
  **When** it is validated against `machine.schema.json`
  **Then** it passes

- **Given** each shape-related failing case in `tests/validation_test.rs`
  **When** it is validated against the schema
  **Then** the schema rejects it, or it is listed as one only `decree check` can catch, with the reason

- **Given** a machine file opened in VS Code with the Red Hat YAML extension
  **When** the `$schema` line is present
  **Then** keys complete and a misspelled key is underlined (say how you checked this, or that it could not be checked here)

- **Given** a project whose `.decree/schema/` is missing or stale
  **When** `decree check` runs
  **Then** it warns and exits 0, and `decree schema` fixes it
