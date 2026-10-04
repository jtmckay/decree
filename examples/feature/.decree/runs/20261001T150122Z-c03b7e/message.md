---
machine: router
id: 20261001T150122Z-c03b7e
parent: 02-upload-quota-per-plan
depth: 1
trigger: invoke
traceparent: 00-670c0ce03a1d9431dd03316e82fde861-631fa3e84f8306f0-01
state: done
---
# Per-plan upload limits

## Requirements

Replace the fixed limit of 10 per minute with a per-plan limit read from
`plans.toml`. Unknown plans use the free-plan limit.

## Acceptance Criteria

- **Given** a key on the `pro` plan with `uploads_per_minute = 100`
  **When** it uploads for the 50th time in a minute
  **Then** the upload succeeds
