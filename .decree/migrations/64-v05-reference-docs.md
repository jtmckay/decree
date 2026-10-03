---
routine: rust-develop
---
# 64: v0.5 cleanup: reference docs and a decision log

## Overview

`docs/0.5-spec.md` is half product reference, half construction plan (tickets, the deletion list, migration numbers, 0.4.2 code references). The rewrite is built, so the plan has done its job. Turn the spec into reference docs that describe decree 0.5 as it is, and keep the reasoning in one decision log.

## Requirements

Read `docs/0.5-spec.md` in full, `docs/0.5-inventory.md`, `docs/spikes/router.md`, `docs/spikes/graph.md`, `docs/routers.md`, `docs/services.md` and `mock/README.md` first.

1. Write `docs/reference/`, one file per subject, describing behaviour only:
   - `README.md`: what decree is, the terms (spec section 1, Terms), the architecture, the `.decree/` layout (there is no configuration file), and an index of the other files.
   - `messages.md`: section 4 (frontmatter, parsing, lifecycle, replies, run lock, migrations).
   - `machines.md`: section 5 (examples, invoke, keys, the SCXML subset with every difference, rules, validation V1–V21). The examples stay identical to the files in `mock/.decree/machines/`.
   - `scripts.md`: section 6.
   - `runs.md`: section 7 (step loop, check, choose: model, routers, sub-machines, choose: person, `events.jsonl` as the public contract).
   - `cli.md`: section 8.
   - `graph.md`: section 9.
   - `observability.md`: section 9a.
   - `standards.md`: section 13.
   Keep every rule and every exact format (field tables, file names, exit codes, error message formats). Drop what only served the construction: rules for implementers ("How to use this spec"), 0.4.2 code references ("reuse 0.4.2's …"), Rust type listings (the code is the truth), sections 10, 11, 12 and 14, and every migration or ticket number.
2. Write `docs/decisions.md`: a decision log in the Architecture Decision Record style (Nygard: context, decision, consequences, one entry per decision). Fold in the router spike's decision record (R1–R10), the graph spike's decision, the inventory's conflicts (C1–C9) and how each was resolved, the answered questions from section 14, and the 0.4 to 0.5 design choices (SCXML, YAML only, no SCXML library, routers as machines, per-visit `onentry`, delete with the last caller). Link evidence by commit hash rather than keeping files.
3. Delete `docs/0.5-spec.md`, `docs/0.5-inventory.md` and `docs/spikes/` (including its PNGs).
4. Replace every reference to the spec (`docs/0.5-spec.md`, "spec section N", "section N" meaning the spec) in `src/`, `tests/`, `README.md`, `mock/`, `docs/`, `examples/` and the skill (`src/templates/skills/`, `.claude/skills/`, `.github/skills/`) with a link or path to the reference file that now holds it. Do not edit `.decree/migrations/`: migrations are immutable.
5. Add `tests/docs_test.rs`:
   - every relative Markdown link in `README.md`, `docs/`, `mock/README.md`, `examples/*/README.md` and the skill resolves to an existing file (and heading anchor, when it has one);
   - every machine example in `docs/reference/machines.md` (a `yaml` block whose first non-comment key is `name:`) is byte-identical to `mock/.decree/machines/<name>.yml`;
   - no file outside `.decree/migrations/` mentions `0.5-spec`.

- Only this migration's scope; migrations 65–68 cover the rest of the cleanup.
- Change no behaviour. If the spec and the code disagree, or the spec is ambiguous, do not guess: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes. The routine fails the run when `STOP` exists.
- Tests build their own `.decree/` in a temp directory and never touch this repository's `.decree/`. No test calls a real LLM or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass. The routine checks this itself after you finish.

## Files to Modify

- docs/reference/ — new
- docs/decisions.md — new
- docs/0.5-spec.md, docs/0.5-inventory.md, docs/spikes/ — delete
- src/, tests/, README.md, mock/README.md, docs/routers.md, docs/services.md, examples/, skills — references only
- tests/docs_test.rs — new

## Acceptance Criteria

- **Given** the repository after this migration
  **When** `rg -n '0\.5-spec|spec section' --glob '!.decree/migrations/**'` runs
  **Then** nothing matches

- **Given** every rule, field table, file name, exit code and error message format in the old spec (except sections 10–12 and 14)
  **When** it is looked up in `docs/reference/`
  **Then** it is there, unchanged in meaning (print a section-to-file table as evidence)

- **Given** `tests/docs_test.rs`
  **When** `cargo test` runs
  **Then** the link, example and `0.5-spec` checks pass

- **Given** `docs/decisions.md`
  **When** it is read
  **Then** it has one entry per decision listed above, each with context, decision and consequences
