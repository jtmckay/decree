---
machine: rust_develop
---
# 90: The skill writes the simplest machine first

## Overview

The user reports that Claude, with the decree skill loaded, makes machines complicated straight away. Asked for a machine, it adds `check` states, `model` decisions, loops with `data` counters, child machines and per-case states before anything needs them. The skill encourages this: its rule "Machines decide, scripts work … a decision is a state of its own, never logic hidden in a script" reads as "turn every `if` into a state".

The user wants the opposite default: **the most basic, naive machine that does the job, until something explicitly warrants more.** "Explicitly" means the user asked for it, or a real run failed in a way the addition fixes.

## Requirements

Read `src/templates/skills/decree/SKILL.md`, its `reference/` files and `docs/reference/machines.md` first.

1. **SKILL.md, a new first rule, "Start with the simplest machine":**
   - Write the most naive machine that does the job: the scripts it needs, one state each, in a straight line (`done` → the next state), ending in `done`. `error` already goes to `failed` implicitly; do not write it.
   - Add nothing else until the user asks for it or a run has shown it is needed. That covers `check`, `model` and `person` decisions, loops and round counters, `data` params, `attempts`, `timeout`, child machines, `emits`, compound states, and one state per case.
   - When an addition is warranted, add the smallest one that solves the problem, and say why in the state's comment (`# the local model fails often: try Claude second`).
   - The cheapest fixes come first, in this order: change the script; `attempts`; a transition on an event the script names; a `check`; a `model` or `person` decision; a child machine.
   - When asked to design a machine, show the simple version and list possible additions in prose as options, not in the YAML.
2. **SKILL.md, replace "Machines decide, scripts work"** with a rule that keeps its point without pushing every `if` into a state:
   - A script may decide *how* to do its one job from its params and environment: which workflow file, which model for this attempt, which flags.
   - A choice of *which step runs next* is a transition. Make it a decision state (`check`, `model`, `person`) only when the step after it differs, and the choice is worth seeing in the graph, testing with `decree check`, or giving to a model or a person.
3. **The worked example** in SKILL.md shows the growth path in three small steps, each with the reason for it:
   1. a straight-line `develop` machine: `implement` then `test`;
   2. after real runs fail at random: `attempts: 3` on `implement`;
   3. after the local model often fails: `attempts: [local, local, claude]`, the script picking the model from `DECREE_ATTEMPT_VALUE`.
   
   Keep the existing "smallest machine" and migration examples. The section stays short.
4. **`reference/machines.md`** in the skill, and **`docs/reference/machines.md`:** a short "Start simple" paragraph near the Style note, saying the same as rule 1 in two or three sentences, for readers without the skill.
5. **The `description` frontmatter** of SKILL.md gains: prefer the simplest machine; add states only when asked or a run shows the need.
6. `CHANGELOG.md`, under Changed: the skill writes the simplest machine first.

- Only this migration's scope: the skill, the two machines reference docs, the changelog. No change to decree's behaviour, the built-in machines or the examples.
- Do not edit `.decree/migrations/` or `.decree/runs/`.
- If anything here contradicts the reference docs in a way you cannot settle, write the question to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- Every machine fragment in the skill passes `decree check` when written out as a whole machine, as `tests/docs_api_test.rs` already checks for the docs. Extend that test to the skill if it does not cover it.
- Print the evidence for each acceptance criterion at the end of your reply.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass.

## Acceptance Criteria

- **Given** the new SKILL.md
  **When** it is read
  **Then** "Start with the simplest machine" is the first rule, it lists what to leave out until needed and the order of fixes, and the rule "Machines decide, scripts work" no longer says that any logic in a script is wrong

- **Given** the worked example
  **When** its three machines are written out
  **Then** each passes `decree check`, the first has only script states in a straight line, and each step's comment gives its reason

- **Given** `decree init` in an empty directory
  **When** `.claude/skills/decree/SKILL.md` is written
  **Then** it is the new text
