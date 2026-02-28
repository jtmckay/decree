# Business Eval — Chain-Based Analysis Pipeline

Evaluate business ideas through a four-step analysis chain:
**market-analysis** → **competitive-landscape** → **financial-model** → **executive-summary**.

Each spec is a different business. Processing a spec triggers the full
evaluation chain automatically.

## What This Demonstrates

- **Chain-based multi-step analysis** — each routine chains to the next
- **Multiple businesses processed independently** — each spec spawns its own chain
- **Accumulated context** — each step passes its output path to the next,
  so later routines build on earlier analyses
- **Custom parameters threaded through chains** — `work_file`, analysis paths,
  `projection_years`

## How It Works

Each spec describes a business idea and routes to `market-analysis` (the
chain entry point). Processing triggers a 4-step chain:

1. **market-analysis.sh** — TAM/SAM/SOM, trends, segments, risks →
   writes `$message_dir/01-market-analysis.md`
2. **competitive-landscape.sh** — Competitor mapping, positioning,
   differentiation → writes `$message_dir/02-competitive-landscape.md`
3. **financial-model.sh** — Revenue projections, unit economics, funding →
   writes `$message_dir/03-financial-model.md`
4. **executive-summary.sh** — Scorecard, strengths/risks, go/no-go →
   writes `$message_dir/04-executive-summary.md`

## Specs

| Spec | Business | Sector |
|------|----------|--------|
| 01 | PetPulse — smart pet health monitoring collar | Pet tech / IoT |
| 02 | GreenRoute — e-cargo bike last-mile delivery | Logistics / sustainability |
| 03 | StudyStream — AI personalized tutoring platform | EdTech / AI |

## Usage

```bash
cd examples/business-eval
decree process
```

Each spec produces a complete evaluation in its chain's run directories
under `.decree/runs/`.
