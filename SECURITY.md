# Security policy

## Supported versions

Only the latest 0.x release gets security fixes.

## Reporting a vulnerability

Report it privately through GitHub: **Report a vulnerability** under this repository's [Security tab](https://github.com/jtmckay/decree/security). Please do not open a public issue.

## The security model

decree is a local automation tool, and it trusts its project directory.

- **Scripts run as you.** decree runs every script directly, as the user who runs decree, with that user's environment and permissions. There is no sandbox and no isolation between runs ([Execution](docs/reference/scripts.md#execution)).
- **Messages are instructions.** A message's body becomes the prompt of the built-in machines' AI calls.
- **The built-in machines let Claude act.** They call Claude with `--permission-mode auto` by default (`CLAUDE_PERMISSION_MODE` changes it), so a message can lead to file edits and commands.

So anyone who can write to `.decree/inbox/`, `.decree/migrations/` or `.decree/cron/`, or run `decree emit`, can direct work on that machine.

## Recommendations

- Process untrusted messages only in a container or VM that holds no credentials it does not need.
- Keep `.decree/` writable only by the user decree runs as.
- Review migrations like code.
- Where commands should not run without review, set `CLAUDE_PERMISSION_MODE` to `acceptEdits` or a stricter mode.
- Keep secrets out of message bodies and script output: both are logged in `.decree/runs/` and `events.jsonl`, and may be shipped to Loki ([observability.md](docs/reference/observability.md)).
- Remove old runs with `decree prune` ([cli.md](docs/reference/cli.md)).

## What decree does do

- Rejects what does not validate before anything runs: `decree check` fails on unknown machine keys (V19) and on invalid messages (M1–M3), `process` and `daemon` refuse to start with an invalid machine, and a message with an unknown machine or param runs nothing ([Validation](docs/reference/machines.md#validation), [Lifecycle](docs/reference/messages.md#lifecycle)).
- Limits which machines a state's scripts may emit to, with the state's `emits`, enforced by `decree emit`. A script can still write to `inbox/` directly, so this guards against mistakes, not against a hostile script.
- Takes a [run lock](docs/reference/messages.md#run-lock), so two processes never step one run.
- Bounds emitted and child chains with `max_depth`.
- Records every action in the run's `events.jsonl` ([runs.md](docs/reference/runs.md)).
