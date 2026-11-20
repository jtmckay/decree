---
machine: rust_develop
---
# 82: `--format json` on every command, and SARIF for `decree check`

## Overview

decree's commands print text for people, so CI systems and AI agents have to scrape it. Decided:

- every command that reports something takes `--format <text|json>`, where JSON is a documented, schema-described document;
- `decree check` also takes `--format sarif`. SARIF 2.1.0 is the OASIS standard that GitHub code scanning, GitLab and Azure DevOps read, so machine errors show up inline in pull requests.

## Requirements

Read `docs/reference/cli.md`, `docs/reference/machines.md` (Validation), `src/cli.rs` and `src/commands/` first.

1. **`--format <text|json>`** on `check`, `status` (both forms), `emit`, `event`, `retry`, `prune`, `graph`, `schema` and `process --dry-run`. `text` is the default and unchanged. With `json`, the command prints exactly one JSON document on stdout, and errors keep going to stderr. Exit codes are unchanged. Documents:
   - `check`: `{ "valid": bool, "errors": [ { "rule", "file", "line"?, "state"?, "message" } ], "warnings": [ { "file", "message" } ] }`, where `rule` is `V1`…`V21` or `M1`…`M3`. Errors with no rule, such as YAML parse errors, use `"rule": null`.
   - `status` with no id: counts by status, and the runs in each group, with the same facts the text shows (id, machine, state, and the running script, wait id and options where they apply). `status <id>`: the run's events, as parsed objects, plus its derived status.
   - `emit` and `event`: `{ "id", "path" }`. `retry`: `{ "id", "state" }`.
   - `prune`: `{ "runs": [ { "id", "machine", "state", "finished" } ], "bytes", "dry_run" }`.
   - `graph` and `schema`: `{ "written": [paths], "removed": [paths] }`.
   - `process --dry-run`: the migrations and inbox messages it would run, with their machines.
   
   `process`, `daemon` and `tail` produce streams, and `init` and `help` are for people, so they take no `--format`; `tail` already prints raw logs.
2. **Schemas** for these documents in `src/templates/schema/v1/cli/` (one per command), written by `decree schema` to `.decree/schema/v1/cli/`, with descriptions, as migration 81 did for the other files.
3. **`decree check --format sarif`** prints a SARIF 2.1.0 log:
   - `tool.driver` has `name: decree`, the version, an `informationUri` (the reference docs), and one `rules` entry per V1–V21 and M1–M3, with `id`, a `shortDescription` taken from the Validation table and a `helpUri` to it;
   - each error is a `result` with `ruleId`, `level: "error"` and `message.text`, plus a `locations[].physicalLocation` with `artifactLocation.uri` (the file, relative to the project root, as `.decree/machines/x.yml`) and `region.startLine` when known;
   - warnings are results with `level: "warning"` and no `ruleId`.
   
   Exit 1 if there are errors, as text and JSON do.
4. **Tests:**
   - every command's JSON validates against its schema, on success and on each error the command can report;
   - `check` JSON lists the same errors as text, for every failing case in `tests/validation_test.rs` (reuse its table);
   - the SARIF has every rule, and each result's `ruleId`, `uri` and `startLine` match the text error.
   
   Validate the SARIF structure against the fields above. The official SARIF schema is not fetched or vendored unless its license allows it; if it does, vendor it under `tests/fixtures/` and validate against it, and say which you did.
5. **Docs:**
   - a "Machine-readable output" section in `docs/reference/cli.md` (the flag, the documents, the schemas, and a GitHub Actions snippet that uploads `decree check --format sarif` with `github/codeql-action/upload-sarif`);
   - the `cli.md` table;
   - the skill (tell a model to use `--format json` instead of parsing text);
   - `help.txt`, the README and `CHANGELOG.md`.

- Only this migration's scope.
- If the reference docs and the code disagree, or a case is not covered here, do not guess: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- Tests build their own `.decree/` in a temp directory and never touch this repository's `.decree/`. No test calls a real LLM or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass.

## Acceptance Criteria

- **Given** each command listed in requirement 1
  **When** it runs with `--format json`
  **Then** stdout is one JSON document that validates against its schema, and the exit code equals the text mode's

- **Given** each failing case in `tests/validation_test.rs`
  **When** `decree check --format sarif` runs
  **Then** each error appears as a result with the matching `ruleId`, file and line
