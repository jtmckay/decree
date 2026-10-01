---
routine: rust-develop
---
# 35: v0.5 M0.2 Swap serde_yaml for serde_norway

## Overview

`serde_yaml` is deprecated and archived. Replace it with `serde_norway` with no behaviour change. Part of the 0.5.0 rewrite specified in `docs/0.5-spec.md` (ticket M0.2).

## Requirements

Read spec sections 1 and 10 (item 15) first.

Replace the dependency and every `serde_yaml` use. Add a test that parses every YAML fixture and config template and compares against committed expected values, and a test that duplicate frontmatter keys are rejected.

- Only this migration's scope; the other v0.5 migrations cover the rest of spec section 11.
- If the code does something the spec does not cover, or the spec is ambiguous here, do not guess: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes. The routine fails the run when `STOP` exists.
- Tests build their own `.decree/` in a temp directory and never touch this repository's `.decree/`. No test calls a real LLM or the network. No new dependencies unless the spec names them.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass. The routine checks this itself after you finish.

## Files to Modify

- Cargo.toml
- every file that uses serde_yaml (rg)

## Acceptance Criteria

- **Given** the updated crate
  **When** `cargo tree -i serde_yaml` runs
  **Then** it reports no such package

- **Given** every existing YAML fixture and template
  **When** they are parsed with serde_norway
  **Then** the values equal the committed expected values

- **Given** frontmatter with the same key twice
  **When** it is parsed
  **Then** parsing fails

- **Given** YAML with a bare key `on` and a value `no`
  **When** it is parsed
  **Then** both are strings, not booleans
