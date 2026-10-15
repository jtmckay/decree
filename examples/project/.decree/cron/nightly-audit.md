---
cron: "0 3 * * *"
machine: develop
---
# No dependency has an advisory that a patched version fixes

## Overview

A vulnerability whose fix is published should not stay in the build overnight.

## Outcome

`cargo audit` reports no advisory that a compatible patched version of the dependency fixes.

## Constraints

- The project's own code does not change, only the versions of its dependencies.

## Acceptance Criteria

- **Given** the project after the run
  **When** `cargo audit` runs
  **Then** it lists no advisory that a compatible patched version fixes
