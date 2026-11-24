---
machine: develop
---
# A16: Documentation sweep before 0.6.0

## Overview

0.6.0 went through four release candidates. A13 to A15 renamed every official plugin to `decree_*`, added `extends`, `source` and plugins added for a need, and moved what was code out of `state/`. Each order updated the docs it touched, and the verify runs kept finding stale text one entry at a time. Before 0.6.0 ships, every document should describe 0.6.0 as it is, in one consistent voice.

This order changes documentation only. It changes no behaviour. It does not bump the version or write the release heading; that is the release commit's job.

## Scope

Read, check against the code and the tests, and fix:

- `README.md`, `SECURITY.md`, and every example's `README.md`.
- `docs/*.md` and `docs/reference/*.md`, except `docs/decisions.md`.
- The decree skill: `plugins/decree_skill.md`, and its installed copies, through `decree install`.
- The `description`s and comments in `plugins/*.md`.
- `CHANGELOG.md` `[Unreleased]`: it must read as the notes for 0.6.0 against 0.5.0. Each entry describes the final state. An entry that only describes how one release candidate differed from an earlier one, for something 0.5.0 never had, is folded into the entry for that feature or removed. What changed from 0.5.0, and what to rewrite, stays.

## What to fix

- **Names that are gone:** official plugins without the `decree_` prefix (`develop`, `lease`, `order`, `ai`...), when they mean the official plugin. A project's own plugin may have any name, so read each mention in context. Also `decree event`, `person:`/`ask:`, `decree process`, `decree plugin ...`, `--official`, `.decree/lib/`, migrations as a current feature, `machines/<name>.yml`, top-level `cron/`, and the domain layout. Each may appear only where a page says what replaced it, for someone converting a 0.5 project.
- **Claims that are not true:** each command, flag, key, path, exit code and printed message a page quotes matches the code. Where they disagree, the code and its tests decide. Fix the doc. If a disagreement looks like a bug in the code, write it to `STOP`, and do not change the code.
- **Links:** every relative link and `#anchor` in the files in scope resolves.
- **Duplication:** a fact written out in full in more than one reference page is said in one and linked from the others, unless a test requires the copy.
- **Voice:** plain, present tense, and what the reader does. Release-candidate history (`rc.1`, `rc.2`...) does not belong in reference pages. It may appear in the CHANGELOG only where a project that ran a release candidate must act (for example, rc.1-rc.2's lease files in `state/`).

## Constraints

- No change to `src/`, `schema/` or test behaviour. A test that pins doc text may be updated with the text.
- `docs/decisions.md` is history: fix only broken links there.
- Only this order's scope.
- If the docs and the code disagree in a way you cannot settle, write the question to a file named `STOP` in the run directory and end without further changes.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass, and `decree check` passes in every example directory.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply.

## Acceptance Criteria

- **Given** the files in scope
  **When** they are searched for the gone names and paths listed above
  **Then** each remaining match is on a line that says what replaced it, and the reply lists them

- **Given** every relative link and anchor in the files in scope
  **When** each is resolved
  **Then** none is broken (a script or test that checks this, and its output)

- **Given** `CHANGELOG.md` `[Unreleased]`
  **When** it is read as the notes for 0.6.0 against 0.5.0
  **Then** no entry contradicts another or the code, and none describes only a difference between two release candidates of a feature 0.5.0 did not have

- **Given** the README's quick start
  **When** it is followed in a fresh temp directory with the built binary
  **Then** every command it shows succeeds as shown
