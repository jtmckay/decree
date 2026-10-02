---
machine: claude_router
id: 20261001T144327Z-6a1f03
parent: 01-rate-limit-upload
depth: 1
trigger: invoke
state: done
---
# Rate-limit /api/upload

## Requirements

Limit each API key to 10 uploads per minute on `POST /api/upload`. Over the
limit, respond 429 with a `Retry-After` header in seconds.

## Acceptance Criteria

- **Given** a key that has uploaded 10 times in the last minute
  **When** it uploads again
  **Then** the response is 429 with `Retry-After`

- **Given** a key that has uploaded 10 times, the oldest 61 seconds ago
  **When** it uploads again
  **Then** the upload succeeds
