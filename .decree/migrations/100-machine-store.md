---
machine: develop
---
# 100: `store:`, what a machine remembers between runs

## Overview

The newsletter example remembers every link it has sent, in `newsletter/seen.tsv`: `gather` reads it, `deliver` appends to it. Nothing in the machine says so; you find it by reading two scripts. The user wants it obvious, from the machine file, what a machine remembers from run to run, and where.

Prior art: n8n gives each workflow persistent "static data", and Node-RED a flow context saved to disk; the XDG Base Directory spec separates *state* (what a program remembers between runs) from data and cache. decree cannot stop a script from writing elsewhere, so this is a standard place, a declaration, and a convention, with the checks that are possible.

It is called **store**, not "state", because "state" already means a machine's state (`DECREE_STATE`).

## Requirements

1. **The folder.** `.decree/store/<machine>/` holds what machine `<machine>` keeps between runs.
   - Every script (invoke, `onentry`, `onexit`, any phase) gets `DECREE_STORE`, the absolute path of its machine's store folder; decree creates it before the script runs. A child machine's scripts get the child machine's folder.
   - It survives runs: `decree prune` never touches it, and nothing else in decree deletes it.
   - Not committed: `decree init`'s `.decree/.gitignore` lists `store/`, beside `runs/` and `inbox/`.
   - `docs/reference/README.md` (file layout), `scripts.md` (the variable, and a short "Store" section).
2. **The declaration**, an optional root key:
   ```yaml
   store:
     seen.tsv: Links already sent, so an item is never in two issues. gather reads it; deliver appends.
   ```
   - A map of names to descriptions. A name is a file or folder directly in the store folder: `^[A-Za-z0-9][A-Za-z0-9._-]*$`, no `/`. A description is a non-empty string.
   - `machine.schema.json` (with a description and this example), `machines.md` (root keys table, and a short "Store" section: what goes in the store, and what does not, namely outputs for people, which go wherever the user wants).
   - **`decree graph`** shows it as a note on the machine: `store: seen.tsv`.
3. **Checks.**
   - The schema checks the key's shape (V-rule as appropriate).
   - **A warning, not an error,** from `decree check`: a file or folder in `.decree/store/<machine>/` that the machine's `store:` does not declare (`store/newsletter/cache.json is not declared in newsletter's store:`), and a store folder with no machine. Warnings do not change the exit code, as today's schema warnings.
   - No other enforcement: a script writing outside its store is not detectable, and the docs say so plainly.
4. **The convention**, in the skill (one rule) and `docs/reference/scripts.md`: anything a script keeps between runs goes in `$DECREE_STORE` and is declared under `store:`, with what it is and which states read or write it. A run's own files go in `$DECREE_RUN_DIR`; shared code and config in `$DECREE_LIB`.
5. **`examples/newsletter`:** `seen.tsv` moves to `$DECREE_STORE/seen.tsv` and is declared under `store:`; `NEWSLETTER_DIR` then holds only the issues. Update its README, scripts, tests and graph.
6. **Tests:** `DECREE_STORE` per machine, including a child machine; the folder is created; `decree prune` leaves it; the schema rejects a name with `/` and an empty description; the undeclared-file warning and the orphan-folder warning, with exit 0; the graph note; `decree init`'s `.gitignore`.
7. `CHANGELOG.md` "Added".

- Only this migration's scope.
- Never edit `.decree/migrations/`, `.decree/runs/` or `.decree/processed.md`, not even by a search and replace across the repository: exclude them from every bulk edit.
- If anything here contradicts the reference docs in a way you cannot settle, write the question to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- No test calls a real model or the network. No new dependencies.
- Print the evidence for each acceptance criterion at the end of your reply.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass, and `decree check` passes in every example directory.

## Acceptance Criteria

- **Given** a machine `m` whose script writes `$DECREE_STORE/count`
  **When** it runs twice
  **Then** `.decree/store/m/count` exists after both runs, and `decree prune --older-than 0s` leaves it

- **Given** `store: { seen.tsv: … }` and a file `.decree/store/m/other.txt`
  **When** `decree check` runs
  **Then** it exits 0 and warns that `other.txt` is not declared

- **Given** `store: { "a/b": x }`
  **When** `decree check` runs
  **Then** it exits 1 naming the key

- **Given** `examples/newsletter`
  **When** its graph and machine are read
  **Then** the machine declares `seen.tsv` under `store:`, the graph shows `store: seen.tsv`, and the scripts read and write `$DECREE_STORE/seen.tsv`
