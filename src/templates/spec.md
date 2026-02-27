# Spec Template

Each spec is a self-contained unit of work. Specs are immutable — once created,
they are processed exactly once and never modified.

## Format

---

## routine: develop

# NN: Title

## Overview

Brief description of what this spec accomplishes.

## Requirements

Detailed technical requirements.

## Files to Modify

- path/to/file.rs — description of changes

## Acceptance Criteria

Write acceptance criteria as BDD-style **Given / When / Then** statements.
Each criterion describes a single testable behaviour that can be directly
translated into an automated test.

- **Given** [a precondition or initial state]
  **When** [an action or event occurs]
  **Then** [an observable, verifiable outcome]

### Guidelines

- One behaviour per criterion — if you need "And" more than once, split
  into separate criteria.
- **Given** sets up state: configuration, data, environment. Be specific
  enough that a test can reproduce it.
- **When** is a single action: a command invocation, a function call, a
  user interaction.
- **Then** is an assertion: what changed, what was produced, what was
  returned. Must be objectively verifiable — no "should work correctly".
- Cover the happy path, key error cases, and edge cases.
- Each criterion maps to one or more test functions. Name tests after the
  scenario they verify.

### Example

- **Given** a project with no `.decree/` directory
  **When** the user runs `decree init`
  **Then** `.decree/` is created with subdirectories: `routines/`, `plans/`,
  `cron/`, `inbox/` (with `done/`), `runs/`, `venv/`

- **Given** the model file does not exist at the configured path
  **When** `decree init` finishes provider selection
  **Then** the user is prompted to download the model
  **And** if declined, the manual download URL is printed and init continues

- **Given** `decree init` has already been run in this directory
  **When** the user runs `decree init` again
  **Then** existing files are not overwritten and a warning is printed

## Rules

- **Naming**: `NN-descriptive-name.spec.md` (e.g., `01-add-auth.spec.md`)
- **Frontmatter**: Optional YAML with `routine:` field (defaults to develop)
- **Ordering**: Alphabetical by filename determines execution order
- **Immutability**: Never edit a processed spec — create a new one instead
- **Self-contained**: Each spec should be independently implementable
- **Day-sized**: Each spec should be completable in one day or less of
  focused work
- **Testable**: Every acceptance criterion must be verifiable by an
  automated test

This file is used as context during `decree plan` when generating specs.
