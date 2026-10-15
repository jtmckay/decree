---
machine: develop
---
# 112: A consensus router, as a recipe

## Overview

A beta user's `verify` passed a change because one local model said `VERDICT: pass`, and asked for decisions that combine several models by a rule (all agree, majority, a set number), with a split going to a person. The user chose a recipe, not a new decision kind: a router is already a replaceable machine (D38, D47), so a consensus router needs nothing new in decree. The recipe also says where it does not apply: `verify` is a script, so a second opinion there is the script calling `ai` twice, a choice of how.

## Requirements

1. **A section in `docs/routers.md`**, "Consensus of several routers": a router machine (`consensus_router`) whose states ask each of N routers in turn (child machines, `invoke: { machine: … }`, each given the same `request.json`), then a script that combines their `reply.json` files by `QUORUM` (a data entry: the number of matching answers needed) and writes the router's own `reply.json`: the agreed option with a confidence from the share that agreed, or `unsure` on a split, so the calling `model` state's `min_confidence` / `unsure` transition sends it to a `person`. Built from existing pieces only, written against the schemas (`request.schema.json`, `reply.schema.json`). Start with the simplest machine that does this, per the skill.
2. **Where it does not apply**: a short paragraph: a script step that wants a second opinion (a `verify` reading `VERDICT:`) calls `ai` twice with different `ai_cli` or models and fails unless both pass; that stays in the script.
3. **Tested from the page**, as `docs/escalation.md` is in `tests/develop_test.rs`: a test extracts the recipe's machine and script, runs it with stub routers (scripts answering fixed options), and checks the result. `decree check` passes on the recipe.
4. **The skill** (`src/templates/skills/decree/SKILL.md`, then `decree skill` refreshes `.claude/skills/decree/`): the "Route with a typed router" rule ends with one sentence: to combine several routers by a quorum, use the consensus recipe in `docs/routers.md` (a router machine, no new decision kind). No built-in machine changes.

- Only this migration's scope.
- Never edit `.decree/migrations/`, `.decree/runs/` or `.decree/processed.md`, not even by a search and replace across the repository: exclude them from every bulk edit.
- If the reference docs and this migration disagree in a way you cannot settle, write the question to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- No test calls a real model or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass, and `decree check` passes in every example directory.

## Acceptance Criteria

- **Given** the recipe's machine and script from `docs/routers.md`, three stub routers answering `a`, `a`, `b`, and `QUORUM=2`
  **When** a `model` state uses it as its router
  **Then** the state takes `a`

- **Given** stubs answering `a`, `b`, `c` and `QUORUM=2`
  **When** the same state runs
  **Then** the state takes its `unsure` transition

- **Given** the recipe's files in a project
  **When** `decree check` runs
  **Then** it passes
