---
machine: develop
---
# 113: Messages describe the outcome, not the steps

## Overview

A message is a spec for an agent. When it says *how* (which function to change, which files to edit, in what order), it ties the agent to one implementation, and when the code differs from what the author pictured, the run stops or the agent forces the plan anyway. Migration 110 stopped for exactly that reason: it asked for a warning "with its own rule id, following the existing warnings", and the existing warnings have no rule ids. An outcome ("`decree check` shows this warning and still exits 0, in text, JSON and SARIF") would have had nothing to conflict with. decree already works this way for the environment: `infra` declares the desired state and `apply` reaches it, as Ansible and Terraform do. The user wants every message to do the same: at the highest level, describe the desired outcome and leave the steps to the agent. Prior art: BDD's Given / When / Then (already decree's acceptance criteria), declarative configuration (desired state, not commands).

## Outcome

- Anyone who reads decree's guidance on writing a message (the skill that `decree init` and `decree skill` write, its reference files, and the reference docs) finds a rule, beside "one concern per migration": **describe the outcome, not the steps**. A message says what is true when it is done and why; the agent chooses how.
- The rule draws the line plainly:
  - **Outcome** (belongs in a message): behaviour a user or another program can observe; the public contract (command and flag names, file names, formats and locations, exit codes, messages printed); what must not change; why the change is wanted.
  - **Steps** (left out): which functions, modules or files to change, in what order, which data structures. One is named only as a constraint ("no new dependencies", "decree itself runs no git command"), never as a suggestion.
  - A test a writer can apply to any sentence: if a different correct implementation would fail it, it is a step.
- The guidance gives the shape of a message body: **Overview** (why), **Outcome** (what is true afterwards), **Constraints** (what must hold or not change), **Acceptance Criteria** (Given / When / Then, observable). It shows one short pair, a step-written sentence and its outcome-written form, using migration 110's warning as the example.
- Every example message decree ships or shows (the skill's worked example, the README, the examples) follows the rule and the body shape. Recorded runs that copy a message stay consistent with it.
- What qa's triage writes for the developer states the broken invariant and the expected behaviour, and proposes no fix.

## Constraints

- Only guidance and examples change: no change to how decree runs, validates or reads a message. A message with any other headings, including the old `## Requirements`, stays valid and runs as before.
- The skill in this repository's `.claude/skills/decree/` is what `decree skill` writes, byte for byte.
- Only this migration's scope.
- Never edit `.decree/migrations/`, `.decree/runs/` or `.decree/processed.md`, not even by a search and replace across the repository: exclude them from every bulk edit. Test fixtures under `tests/fixtures/` keep their recorded messages.
- If the reference docs and this migration disagree in a way you cannot settle, write the question to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- No test calls a real model or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass, and `decree check` passes in every example directory.
- CHANGELOG under Changed.

## Acceptance Criteria

- **Given** an empty directory
  **When** `decree init` runs and its skill is read
  **Then** the rules include "describe the outcome, not the steps", with the outcome/steps line and the different-implementation test, and the worked example message has `## Outcome` and no `## Requirements`

- **Given** the skill's reference on writing a message
  **When** it is read
  **Then** it gives the Overview / Outcome / Constraints / Acceptance Criteria shape and the step-versus-outcome pair from migration 110

- **Given** the repository
  **When** `rg -n --hidden '^## Requirements' --glob '!.decree/migrations/**' --glob '!.decree/runs/**' --glob '!tests/fixtures/**' --glob '!.git/**'` runs
  **Then** it matches nothing

- **Given** a message with `## Requirements` and no `## Outcome`
  **When** `decree check` and `decree process` run
  **Then** both accept and run it as before

- **Given** this repository
  **When** `decree skill` runs
  **Then** `git status --porcelain -- .claude/skills` is empty
