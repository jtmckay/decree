# Business Eval — Chain-Based Analysis Pipeline

Evaluate business ideas through a four-step analysis chain:
**market_analysis** → **competitive_landscape** → **financial_model** → **executive_summary**.

Each spec is a different business. Processing a spec triggers the full
evaluation chain automatically.

## What This Demonstrates

- **Chaining with `decree emit`** — the last state of each machine queues a
  message for the next machine; `emits:` in the machine names which machine
  it may queue, and [`.decree/graph/system.md`](.decree/graph/system.md) draws
  the chain
- **Multiple businesses processed independently** — each spec spawns its own
  chain, and the chain finishes before the next spec starts (a migration
  waits for the inbox to drain)
- **Accumulated context** — each step passes its report's path to the next
  as a param, so later machines build on earlier analyses
- **Typed parameters threaded through chains** — analysis paths and
  `projection_years` are each machine's `data`

## How It Works

Each spec describes a business idea in its body and names `market_analysis`
(the chain entry point). Every machine runs `precheck` (claude is
installed), `analyze` (claude writes the report into the run folder) and
`check_report` (the report exists); all but the last then run `hand_off`,
which emits the next message with the same idea as its body:

1. **market_analysis** — TAM/SAM/SOM, trends, segments, risks →
   writes `runs/<id>/01-market-analysis.md`
2. **competitive_landscape** — Competitor mapping, positioning,
   differentiation → writes `runs/<id>/02-competitive-landscape.md`
3. **financial_model** — Revenue projections, unit economics, funding →
   writes `runs/<id>/03-financial-model.md`
4. **executive_summary** — Scorecard, strengths/risks, go/no-go →
   writes `runs/<id>/04-executive-summary.md`

`precheck` and `check_report` are shared, in `.decree/scripts/`; each
machine's `analyze` and `hand_off` are its own, in `.decree/scripts/<machine>/`.

## Specs

| Spec | Business | Sector |
|------|----------|--------|
| 01 | PetPulse — smart pet health monitoring collar | Pet tech / IoT |
| 02 | GreenRoute — e-cargo bike last-mile delivery | Logistics / sustainability |
| 03 | StudyStream — AI personalized tutoring platform | EdTech / AI |

## Usage

```bash
cd examples/business-eval
decree check
decree process
decree status
```

`decree` must be on `PATH`, since `hand_off` calls `decree emit`. Each spec
produces a complete evaluation across the run folders of its chain under
`.decree/runs/`; `decree status <id>` shows one run's steps.

To evaluate a new idea without writing a migration:

```bash
printf '# Idea\n\nA subscription service for houseplant care.\n' | decree emit --machine market_analysis
decree process
```
