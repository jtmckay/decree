---
routine: rust-develop
---

# 18: YAML array/object env vars

## Overview

Custom frontmatter fields whose values are YAML arrays or mappings are
currently silently dropped by `value_as_env_string`. Serialize them to
compact JSON strings so routines can consume structured inputs.

## Requirements

Currently `value_as_env_string` silently drops any custom frontmatter field
whose value is a YAML array or mapping. Serialize those values to a compact
JSON string instead, so routines can consume structured inputs.

Example frontmatter:

```yaml
input_image:
  - input_image: some_path.png [output]
    output_prefix: some_prefix
```

Must set env var:

```
input_image=[{"input_image":"some_path.png [output]","output_prefix":"some_prefix"}]
```

This applies wherever custom fields are passed as env vars: routine execution
(`execute_routine`) and hook execution (`run_hook_with_config` for message-
scoped hooks). Add a dedicated unit test for the array case.

## Files to Modify

- `src/message.rs` — array/object JSON serialization in `value_as_env_string`

## Acceptance Criteria

- **Given** a message with a custom field whose value is a YAML array of
  objects (e.g., `input_image: [{input_image: path.png, output_prefix: pfx}]`)
  **When** the routine is executed
  **Then** the env var `input_image` is set to the compact JSON string
  `[{"input_image":"path.png","output_prefix":"pfx"}]`

- **Given** a message with a custom field whose value is a YAML mapping
  (e.g., `options: {key: val}`)
  **When** the routine is executed
  **Then** the env var `options` is set to the JSON string `{"key":"val"}`

- **Given** a message with a scalar custom field (string, number, bool)
  **When** the routine is executed
  **Then** the env var is set to the scalar's string representation
  (unchanged from current behaviour)

- **Given** a unit test that parses a migration with array frontmatter
  ```yaml
  input_image:
    - input_image: some_path.png [output]
      output_prefix: some_prefix
  ```
  **When** `value_as_env_string` is called on the parsed value
  **Then** it returns `[{"input_image":"some_path.png [output]","output_prefix":"some_prefix"}]`
