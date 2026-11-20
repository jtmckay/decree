---
machine: rust_develop
---
# 81: A versioned JSON Schema for every file decree reads or writes

## Overview

`decree schema` writes schemas for machines and message frontmatter. The other files of the contract have only prose:

- `events.jsonl`, which dashboards and pipelines consume;
- `request.json` and `reply.json`, which routers in any language read and write.

The schemas also carry no version, so a future breaking change could not be told apart from a mistake. Decided:

- publish a schema for every file;
- version them in the path, `v1`;
- state the versioning rule.

## Requirements

Read `docs/reference/` (all of it), `src/templates/schema/` and `tests/schema_test.rs` first.

1. **Versioned location.** Schemas move to `src/templates/schema/v1/`, and `decree schema` writes them to `.decree/schema/v1/`.
   - Each `$id` is `https://raw.githubusercontent.com/jtmckay/decree/main/src/templates/schema/v1/<file>`.
   - Machine files point at `../schema/v1/machine.schema.json`. Update every machine in `examples/` and `src/templates/`, the docs and `docs_test`.
   - `decree check` warns when `.decree/schema/` holds anything other than what `decree schema` writes, including the old unversioned files, and `decree schema` removes those.
2. **New schemas** in `v1/`, draft 2020-12, with a `description` on every property taken from the reference docs:
   - `events.schema.json`: one line of `events.jsonl`. It has the common fields (`v` is the constant 1), then a `oneOf` per `type`, with each type's fields as `docs/reference/runs.md` lists them, required or optional as documented.
   - `request.schema.json`: `request.json`, including `reply_schema`.
   - `reply.schema.json`: the general shape of `reply.json` (`event` required; `confidence`, `reason`, `probabilities` optional). `reply_schema` in each request stays the exact per-request schema.
3. **Versioning rule**, in a new section of `docs/reference/README.md`:
   - Within `v1`, changes are additive only: new optional fields, new event types, new optional keys.
   - A rename, a removal, or a change of meaning is `v2`: a new directory and a new `v` value in events.
   - decree 0.x may still change `v1` before 1.0, and every such change is listed in the changelog.
   
   Add a `CHANGELOG.md` at the repository root in the Keep a Changelog format, with a 0.5.0 entry that summarises the 0.5 contract (link the reference) and the changes since: migrations 71–80 and this one.
4. **Tests, holding the code to the schemas:**
   - every line of every `events.jsonl` in `examples/` validates against `events.schema.json`;
   - every line written by the property test in `tests/interpreter_props.rs`, which exercises the interpreter widely, validates too;
   - every `request.json` and `reply.json` in `examples/` validates against its schema;
   - a `request.json` written by a test run validates;
   - every schema is a valid draft 2020-12 schema, and every property has a `description`;
   - the existing machine and message schema tests keep passing at the new path.
5. **Docs:** list the schemas and what each covers in `docs/reference/README.md`. Link each from the section that documents its file (`runs.md` for events and requests, `machines.md`, `messages.md`). Update the skill.

- Only this migration's scope.
- If the reference docs and the code disagree, or a case is not covered here, do not guess: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- Tests build their own `.decree/` in a temp directory and never touch this repository's `.decree/`, except running `decree schema` in this repository and the examples to refresh their committed schema files. No test calls a real LLM or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass.

## Acceptance Criteria

- **Given** every `events.jsonl` line in `examples/` and every line the property test writes
  **When** it is validated against `events.schema.json`
  **Then** it passes

- **Given** a project with the old unversioned schema files
  **When** `decree check` and then `decree schema` run
  **Then** check warns, and schema leaves only `.decree/schema/v1/`

- **Given** `CHANGELOG.md`
  **When** it is read
  **Then** it has a 0.5.0 entry and the versioning rule is linked from it
