---
routine: rust-develop
---
# 41: v0.5 M2.1 Script resolver

## Overview

Resolve a script name to exactly one executable. Comes before the validator because rule V12 uses it. Part of the 0.5.0 rewrite specified in `docs/0.5-spec.md` (ticket M2.1).

## Requirements

Read spec sections 1 and 3 (Shared machines), 6 (Resolution) first.

Create `src/runtime.rs` with the section 6 resolver: `scripts/<machine>/`, then `scripts/`, in the project and then in `shared_source`; the first directory with a match wins.

- Only this migration's scope; the other v0.5 migrations cover the rest of spec section 11.
- If the code does something the spec does not cover, or the spec is ambiguous here, do not guess: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes. The routine fails the run when `STOP` exists.
- Tests build their own `.decree/` in a temp directory and never touch this repository's `.decree/`. No test calls a real LLM or the network. No new dependencies unless the spec names them.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass. The routine checks this itself after you finish.

## Files to Modify

- src/runtime.rs — new
- src/lib.rs

## Acceptance Criteria

- **Given** `scripts/x` executable
  **When** `x` is resolved for machine `m`
  **Then** that file is returned

- **Given** `scripts/x.sh` executable
  **When** `x` is resolved
  **Then** that file is returned

- **Given** both `scripts/m/x.sh` and `scripts/x.sh`
  **When** `x` is resolved for `m`, then for machine `n`
  **Then** `m` gets `scripts/m/x.sh` and `n` gets `scripts/x.sh`

- **Given** `x` only in `shared_source/scripts/`, then also in the project's `scripts/`
  **When** `x` is resolved
  **Then** the shared file first, then the project file

- **Given** both `x.sh` and `x.py` in one directory
  **When** `x` is resolved
  **Then** it fails naming both

- **Given** `x.sh` without an execute bit
  **When** `x` is resolved
  **Then** it fails as not executable

- **Given** no matching file
  **When** `x` is resolved
  **Then** it fails as missing
