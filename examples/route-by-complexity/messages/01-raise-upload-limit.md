---
machine: develop_by_size
---
# Raise the upload limit to 25 MB

## Overview

Users need to upload files of up to 25 MB, and uploads over 10 MB are rejected today.

## Outcome

An upload of up to 25 MB is accepted; a larger one is still rejected.

## Constraints

- The limit stays where it is set, in `src/config.rs`; nothing else changes.

## Acceptance Criteria

- **Given** an upload of 20 MB
  **When** it is posted
  **Then** it is accepted

- **Given** an upload of 30 MB
  **When** it is posted
  **Then** it is rejected
