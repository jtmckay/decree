---
machine: develop
---
# A10: A cron file can fire when the daemon starts (`@reboot`)

## Overview

Some work should happen whenever the long-running process comes up: warm a model, check that services answer, catch up on what was missed while the host was down. Today a cron file only fires on a time schedule. existential works around this with an `on-boot` flow and a `boot` machine. The user wants a cron file to be able to say "whenever the daemon starts". For a daemon that runs from boot to shutdown, that means "on boot".

cron already has a word for this. In Vixie cron (cronie and the BSDs), `@reboot` means "run once, at startup", and it fires when the cron daemon starts, not when the kernel boots. decree uses the same spelling with the same meaning. Then anyone who knows crontab can read it, and `decree daemon` is the cron daemon.

## Outcome

- A cron file whose `cron:` is `@reboot` is written to `inbox/` once each time `decree daemon` starts, at the daemon's first pass. It goes through the same writer as any other cron file, with `trigger: cron`, and the same rules apply: keys copied, `cron` not copied, and `--machine`/`--exclude-machine` (or `DECREE_MACHINES`/`DECREE_EXCLUDE_MACHINES`) decide whether this daemon fires it.
- It fires on every start, including `decree daemon --retry`. Restarting a daemon that crashes and is restarted by a supervisor therefore queues it again each time. The docs say so, and say that a script that must not repeat should check its own result first, as every script should.
- It fires only at start: a `@reboot` cron file added while the daemon runs waits for the next start, as in Vixie cron. `decree process` never fires it, just as it fires no cron file.
- When the queue is held (an attention file holds, or `decree stop` was used), the message is still queued at start and runs once the hold is gone.
- `decree check` accepts `@reboot` and validates the file as it validates any cron file.
- `decree status --cron` lists a `@reboot` file with "daemon start" in place of its next fire time. `--format json` gives it no next time and marks it as firing at daemon start, and `schema/v1/cli/status.schema.json` describes that.
- Other cron expressions behave exactly as before.

## Constraints

- `@reboot` is the only new spelling. No `daemon` alias, and no other new key in the cron file.
- decree keeps no record of past starts: "once per start" means once per `decree daemon` process.
- Only this migration's scope.
- Never edit `.decree/migrations/`, `.decree/runs/` or `.decree/processed.md`, not even by a search and replace across the repository: exclude them from every bulk edit.
- If the reference docs and this migration disagree in a way you cannot settle, write the question to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- No test calls a real model or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass, and `decree check` passes in every example directory.
- Documentation:
  - the cron files section of `docs/reference/messages.md` and the decree skill template show `@reboot` with a short example;
  - CHANGELOG lists the change under Added;
  - `docs/decisions.md` gets a decision citing Vixie cron's `@reboot` and saying why it is not spelled `daemon`.

## Acceptance Criteria

- **Given** a cron file with `cron: "@reboot"` for machine `boot`
  **When** `decree daemon` starts and makes several passes
  **Then** exactly one message for `boot` with `trigger: cron` is queued

- **Given** the same project
  **When** the daemon is stopped and started again
  **Then** a second message is queued

- **Given** a `@reboot` cron file
  **When** `decree process` runs
  **Then** nothing is queued from it

- **Given** a daemon running with no `@reboot` file
  **When** one is added to `cron/`
  **Then** nothing is queued from it until the daemon starts again

- **Given** a `@reboot` cron file for `boot` and a daemon started with `--exclude-machine boot`
  **When** it starts
  **Then** nothing is queued from it

- **Given** a holding attention file
  **When** the daemon starts
  **Then** the `@reboot` message is queued, and it runs only after the attention file is deleted

- **Given** a `@reboot` cron file
  **When** `decree check` and `decree status --cron --format json` run
  **Then** check passes, and the status entry validates against the schema and says it fires at daemon start
