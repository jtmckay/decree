---
machine: rust_develop
---
# 87: Every doc and example matches the 0.5 API, and tests keep it that way

## Overview

The 0.5 interface changed many times between migrations 71 and 86: machine keys, durations, the event file, schemas, `--format`, tracing, and `decree process --retry`. Each migration updated the docs it knew about, and a sweep for known old terms still finds leftovers:

- `docs/reference/README.md` has a `max_retries` row, a 0.4 name;
- `docs/services.md` uses `use_comfyui`/`use_llm`, while `examples/tmux-services/` uses `use_comfy`/`without_ollama`.

A sweep only finds what someone already knows to look for. This migration reviews every document and example against the code, fixes what disagrees, and adds tests that check docs and examples against the binary itself, so the next interface change fails `cargo test` instead of leaving stale docs.

## Requirements

Read `docs/reference/` in full, `README.md`, `docs/routers.md`, `docs/services.md`, `SECURITY.md`, every `examples/*/README.md`, the decree skill (`src/templates/skills/decree/`), `src/templates/help.txt`, `src/cli.rs` and `tests/README.md` first.

1. **Review and fix.** Read every document and example listed above against the code: commands, flags, defaults, exit codes, error messages, file names, environment variables, machine keys, event fields, schema paths. Fix every statement that disagrees with what decree does today. The code is the reference. If the code looks wrong instead of the doc, do not change the code: list it under "Possible code issues" in your reply. Known items:
   - the `max_retries` row: name the setting for what it is today, as "Retries";
   - `docs/services.md`'s systemd example: use the `use_<service>`/`without_<service>` naming of `examples/tmux-services/`.
   
   `CHANGELOG.md`, `docs/decisions.md` and `docs/code-review.md` are history: change them only where they state something false about 0.5.0.
2. **Tests that check docs against the binary** (`tests/docs_api_test.rs`):
   - **Commands and flags.** Every command written as `decree <command> …` in the documents above, in shell blocks and in inline code, names a real command, and every `--flag` it uses is accepted by that command. Check against the built binary: each subcommand's `--help` output, or clap's command definition, whichever is simpler. Ignore placeholders such as `<id>`.
   - **Machines.** Every YAML block in those documents that is a whole machine (it has `name:` and `states:`) passes `decree check` in a temp project. The test creates an executable stub for every script name the machine uses, and any child or router machine it names. A block that is a fragment (states only) is validated against the machine schema's state definition, or skipped with a reason in a small allow list in the test.
   - **Messages.** Every frontmatter example (a block starting with `---` that names `machine:`, or `to:` and `event:`) validates against `message.schema.json`.
   - **Events.** Every JSON object in `docs/reference/runs.md` and `docs/reference/observability.md` that has `"type"` and is shown as an `events.jsonl` line validates against `events.schema.json`, as far as it is complete. Where examples are elided with `…`, skip them, listing them in the test.
   - **Environment.** Every `DECREE_*` variable named anywhere in the documents, scripts and templates is in `docs/reference/scripts.md`'s Environment table, and every variable in that table is set by `src/runtime.rs`. Find them in the source the way the test finds the others, not by a hand list.
3. **Fix what the tests find,** in the documents, not by weakening the tests. Each allow-list entry needs a one-line reason.
4. **Update** `tests/README.md` (the new file), and add a `CHANGELOG.md` "Fixed" entry that summarises the corrections.

- Only this migration's scope: documents, examples, the skill, `help.txt` and tests. No change to decree's behaviour.
- If the reference docs and the code disagree in a way you cannot settle from the code, do not guess: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- No test calls a real LLM or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log, with the list of corrections made and any "Possible code issues".
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass.

## Acceptance Criteria

- **Given** the documents listed above
  **When** `tests/docs_api_test.rs` runs
  **Then** every command and flag exists, every whole machine passes `decree check`, every frontmatter and event example validates, and the environment variables agree with the code, with any skips listed and explained

- **Given** a doc that mentions a flag decree does not have (try `decree process --no-such-flag` in a scratch copy)
  **When** the test runs
  **Then** it fails naming the file and the flag

- **Given** the review
  **When** it is done
  **Then** the reply lists every correction, and any statement where the code, not the doc, looks wrong
