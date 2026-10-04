---
machine: develop_by_size
---
# Raise the upload limit to 25 MB

The upload limit in `src/config.rs` is 10 MB. Raise it to 25 MB.

## Acceptance Criteria

- **Given** an upload of 20 MB
  **When** it is posted
  **Then** it is accepted
