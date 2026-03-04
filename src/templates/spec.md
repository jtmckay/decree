# Spec Template

Each migration is a self-contained unit of work. Migrations are immutable —
once created, they are processed exactly once and never modified.

## Format

    ---
    routine: develop
    ---
    # NN: Title

    ## Overview
    Brief description of what this migration accomplishes.

    ## Requirements
    Detailed technical requirements.

    ## Acceptance Criteria
    - [ ] Criterion 1
    - [ ] Criterion 2

## Rules

- **Naming**: `NN-descriptive-name.md` (e.g., `01-add-auth.md`)
- **Frontmatter**: Optional YAML with `routine:` field (defaults to develop)
- **Ordering**: Alphabetical by filename determines execution order
- **Immutability**: Never edit a processed migration — create a new one
- **Self-contained**: Each migration should be independently implementable
