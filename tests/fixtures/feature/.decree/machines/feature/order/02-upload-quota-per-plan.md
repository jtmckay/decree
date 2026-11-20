---
machine: feature
params:
  max_rounds: 3
---
# Per-plan upload limits

## Requirements

Replace the fixed limit of 10 per minute with a per-plan limit read from
`plans.toml`. Unknown plans use the free-plan limit.

## Acceptance Criteria

- **Given** a key on the `pro` plan with `uploads_per_minute = 100`
  **When** it uploads for the 50th time in a minute
  **Then** the upload succeeds
