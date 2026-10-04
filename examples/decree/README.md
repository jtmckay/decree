# Decree — Built with Decree

Decree is built using itself. Every change to the CLI, from the parser to the
interpreter, the daemon and these examples, starts as a migration in this
repository's own [`.decree/migrations/`](../../.decree/migrations/) and runs
through `decree process`.

## How It Works

Each migration is a spec: an overview, requirements, the files to change and
acceptance criteria. It names the machine that runs it:

```markdown
---
machine: rust_develop
---
# 71: One shape per machine key
...
```

`rust_develop` is the machine `decree init` writes for Rust projects:
`implement` hands the spec to an AI agent, which works in small steps logged
to `progress.md`; a `gate` runs `cargo fmt --check`, `cargo clippy` and
`cargo test`; and `qa` runs only if the gate fails. An agent that finds the
spec unclear writes a `STOP` file instead of guessing, and the run fails until
someone answers it.

```bash
decree process         # run the next unprocessed migration
decree status          # check processing progress
decree status <id>     # review one run: its states, scripts and log files
```

`processed.md` lists the migrations that have run, in order; `runs/<id>/`
keeps each run's events and logs.

## What This Demonstrates

- **Incremental, spec-driven development**: each migration builds on the code
  produced by the ones before it, so ordering matters
- **Self-hosting**: the tool's own machines are used to build the tool
- **Reproducibility**: the migrations stay in the repository, in order, next
  to the ledger of the ones that ran
