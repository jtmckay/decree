---
machine: rust_develop
---
# 86: Release docs: no 0.4 pitch, the upgrade note, and a security policy

## Overview

Three documentation gaps to close before 0.5.0 is published:

- `SOW.md` is decree 0.4's pitch, written around routines and `config.yml`, which 0.5 removed.
- The README doesn't say there is no upgrade path from 0.4; only `CHANGELOG.md` does. crates.io still serves 0.4.2, so people will arrive from 0.4.
- There is no security policy. decree runs scripts with the user's full permissions and no sandbox, and its built-in machines run Claude with `--permission-mode auto`. So anyone who can put a message in `inbox/`, `migrations/` or `cron/` can direct an agent on that machine. Users must be told, plainly.

## Requirements

Read `README.md`, `CHANGELOG.md`, `docs/reference/scripts.md` (Execution, Environment), `docs/reference/messages.md` (Lifecycle), `src/templates/ai/claude.sh` and the decree skill first.

1. **Delete `SOW.md`** with `git rm`. Nothing links to it. If any doc, test or template mentions it, remove the mention.
2. **README:** near the install instructions, add a short "Coming from 0.4?" note:
   - 0.5 is a rewrite;
   - there is no upgrade path or upgrade tool, so finish pending 0.4 work with 0.4 first;
   - a 0.4 `.decree/` is not read;
   - link the changelog's 0.5.0 entry.
   
   Two or three sentences.
3. **`SECURITY.md`** at the repository root, in the usual GitHub form, which GitHub links from the repository's Security tab. Short and concrete:
   - **Supported versions:** the latest 0.x release only.
   - **Reporting a vulnerability:** use GitHub's private vulnerability reporting ("Report a vulnerability" under the repository's Security tab). Not a public issue. No email address.
   - **The security model, as it is.** decree runs every script directly, as the user who runs decree, with that user's environment and permissions: no sandbox, no isolation between runs. Messages are instructions: their body becomes the prompt of the built-in machines' AI calls. The built-in machines call Claude with `--permission-mode auto` by default (`CLAUDE_PERMISSION_MODE` changes it), so a message can lead to file edits and commands. Anyone who can write to `.decree/inbox/`, `.decree/migrations/` or `.decree/cron/` (or reach `decree emit`) can therefore direct work on that machine.
   - **Recommendations:**
     - process untrusted messages only in a container or VM with no credentials it does not need;
     - keep `.decree/` writable only by the user decree runs as;
     - review migrations like code;
     - prefer `acceptEdits` or a stricter mode (`CLAUDE_PERMISSION_MODE`) where commands should not run without review;
     - keep secrets out of message bodies and script output, since both are logged in `runs/` and `events.jsonl` and may be shipped to Loki;
     - use `decree prune` to remove old runs.
   - **What decree does do:**
     - unknown keys and invalid messages are rejected before anything runs (`decree check`, M1–M3);
     - `emits` limits which machines a state's scripts may emit to;
     - the run lock prevents two processes stepping one run;
     - `max_depth` bounds emitted and child chains;
     - every action is recorded in `events.jsonl`.
4. **Link `SECURITY.md`** from the README (one line, near the license section) and from `docs/reference/scripts.md`'s Execution section (one sentence). Add a `CHANGELOG.md` entry under 0.5.0 (Added: the security policy; Removed: `SOW.md`).
5. **Tests:**
   - `tests/docs_test.rs` already checks that every relative link resolves; make sure it covers `SECURITY.md`;
   - add a test that `SECURITY.md` names `--permission-mode auto` and `CLAUDE_PERMISSION_MODE`, and that `src/templates/ai/claude.sh` still defaults to `auto`, so the policy and the code cannot drift apart.

- Only this migration's scope.
- If the reference docs and the code disagree, or a case is not covered here, do not guess: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- No test calls a real LLM or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass.

## Acceptance Criteria

- **Given** the repository
  **When** `ls SOW.md` and `rg -n 'SOW' --glob '!.decree/**'` run
  **Then** neither finds anything

- **Given** `SECURITY.md`
  **When** it is read
  **Then** it covers supported versions, private reporting, the security model and recommendations, and its permission-mode statement matches `src/templates/ai/claude.sh` (tested)

- **Given** the README
  **When** it is read
  **Then** it says there is no upgrade path from 0.4, and links the changelog and `SECURITY.md`
