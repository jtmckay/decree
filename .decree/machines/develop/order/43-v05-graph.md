---
routine: rust-develop
---
# 43: v0.5 M1.4 decree graph

## Overview

Draw machines as Mermaid, from the same arena the interpreter will run. Part of the 0.5.0 rewrite specified in `docs/0.5-spec.md` (ticket M1.4).

## Requirements

Read spec sections 1 and 5 (Example), 9; `mock/graph/` first.

Create `src/graph.rs` and the `graph [<machine>]` subcommand exactly as section 9 specifies, printing the section 9 Markdown document, with the viewing instructions in `--help`. Copy `mock/graph/feature.md` to `tests/fixtures/graph/feature.md`. Render no images.

- Only this migration's scope; the other v0.5 migrations cover the rest of spec section 11.
- If the code does something the spec does not cover, or the spec is ambiguous here, do not guess: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes. The routine fails the run when `STOP` exists.
- Tests build their own `.decree/` in a temp directory and never touch this repository's `.decree/`. No test calls a real LLM or the network. No new dependencies unless the spec names them.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass. The routine checks this itself after you finish.

## Files to Modify

- src/graph.rs — new
- src/cli.rs
- src/commands/graph.rs — new
- tests/fixtures/graph/ — new

## Acceptance Criteria

- **Given** the section 5 example
  **When** `decree graph feature` runs
  **Then** stdout equals `tests/fixtures/graph/feature.md` byte for byte

- **Given** two machines where one state `emits` the other, and one cron file
  **When** `decree graph` runs with no argument
  **Then** stdout equals its fixture

- **Given** `mock/` as the project root
  **When** `decree graph <m>` runs for each machine, and `decree graph` with no argument
  **Then** each output equals the matching file in `mock/graph/`

- **Given** `npx` is available
  **When** `npx -y @mermaid-js/mermaid-cli -i tests/fixtures/graph/feature.md -o /tmp/f.md` runs
  **Then** it exits 0 (if `npx` cannot run, say so in the run log)
