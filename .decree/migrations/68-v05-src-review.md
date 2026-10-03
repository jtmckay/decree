---
routine: rust-develop
---
# 68: v0.5 cleanup: review and simplify src/

## Overview

`src/` was built by fourteen migrations, several of which reworked what earlier ones built (decision invokes replaced router states, `process` and `daemon` were merged late). Review it for what that left behind and simplify it, without changing behaviour. The black-box tests from migrations 66 and 67 are the safety net.

## Requirements

Read `docs/reference/` and `tests/README.md` first, then all of `src/`.

Before changing anything, copy `tests/` to `tests-before/` in the run directory (the directory that holds the message file you were given), and print `wc -l` of the non-test code of each module in `src/`. The queue does not commit between migrations, so these are the baseline.

1. **Review.** Write `docs/code-review.md`: for each module, its job in one sentence, and every finding with a file and line. Look for:
   - logic that exists twice, or two ways of doing one thing (paths, event writing, run status, script resolution, message parsing);
   - functions, types or fields that only tests use, or that have one caller and add nothing;
   - names and shapes left from 0.4 or from earlier designs (routines, router states, `cond` strings, dead letters, chains);
   - `pub` items that could be private; modules over about 800 lines of non-test code that hold more than one job;
   - error handling that loses context, `unwrap` or `expect` outside tests, and panics a user could trigger;
   - unit tests that only repeat a black-box test.
2. **Simplify.** Fix every finding that does not change behaviour. Split `src/interpreter.rs` into modules by job (for example the step loop, decisions, child runs, recovery and status, events) if the review shows it holds several. Delete what step 1 found unused. Move a unit test to `tests/` only if it tests behaviour a user can see; keep unit tests of pure functions.
3. Mark each finding in `docs/code-review.md` as fixed, or as kept with the reason.

- Only this migration's scope.
- Change no behaviour: no black-box test in `tests/` changes, except imports and helpers. If a simplification would change behaviour, or a test looks wrong, do not change it: note it in `docs/code-review.md` as kept, or, if it blocks the review, write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end. The routine fails the run when `STOP` exists.
- Tests build their own `.decree/` in a temp directory and never touch this repository's `.decree/`. No test calls a real LLM or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass. The routine checks this itself after you finish.

## Files to Modify

- docs/code-review.md — new
- src/

## Acceptance Criteria

- **Given** the black-box tests in `tests/`
  **When** `diff -r <run dir>/tests-before tests` is read
  **Then** only imports, helpers and moved unit tests changed (print the diff summary)

- **Given** `src/` before and after (print `wc -l` of non-test code per module)
  **When** they are compared
  **Then** the non-test line count did not grow, and no module over about 800 non-test lines holds more than one job

- **Given** `docs/code-review.md`
  **When** it is read
  **Then** every finding has a file, a line, and a status (fixed, or kept with a reason)

- **Given** `rg -n '\.unwrap\(\)|\.expect\(' src --glob '!**/tests.rs'` outside `#[cfg(test)]` code
  **When** each hit is checked
  **Then** each is fixed, or noted in `docs/code-review.md` as unreachable with the reason
