# Decree — Built with Decree

Decree was built using itself. The entire CLI — parser, config system, AI
integration, pipeline engine, checkpoint system, daemon — was produced by
writing spec files and running `decree process`.

## How It Worked

11 spec files were written in order, each describing a piece of the system:

```
specs/
├── 01-init-config-and-cli.spec.md
├── 02-default-templates.spec.md
├── 03-embedded-ai-and-bench.spec.md
├── 04-checkpoint-system.spec.md
├── 05-message-format-and-normalization.spec.md
├── 06-routine-system.spec.md
├── 07-run-process-and-pipeline.spec.md
├── 08-daemon-and-cron.spec.md
├── 09-interactive-planning.spec.md
├── 10-change-review-diff-and-apply.spec.md
└── 11-post-cleanup.spec.md
```

Each spec was processed by the `rust-develop` routine, which hands the spec
to an AI, builds with `cargo build --release`, runs `cargo test`, and has a
QA pass fix any failures — all in one automated cycle.

```bash
decree process     # process next unprocessed spec
decree diff        # review what the AI wrote
decree apply --all # apply changes to the working tree
```

Repeat until all 11 specs are processed. The result is the `src/` directory,
`Cargo.toml`, tests, templates — the complete working tool.

## What This Demonstrates

- **Incremental, spec-driven development** — each spec builds on the code
  produced by prior specs, so ordering matters
- **Self-hosting** — the tool's own pipeline was used to build the tool
- **Review before apply** — every spec's output was diffed and reviewed
  before being accepted into the working tree
- **Reproducibility** — the specs are still in the repo; the same sequence
  could be re-run from a clean slate
