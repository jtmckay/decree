# Sort documents: an escalation ladder

This example files scanned documents as invoices, receipts or other paperwork, and asks for as little as it can: two free checks first, then a small local model, then a large model, and a person only when the large model is unsure but not clueless. It is a decree project with one finished run that climbs every step, recorded with its two router child runs.

Nothing here runs on its own: the routers' scripts call a local classifier service and Claude. Tests hold the snapshot to the [reference](../../docs/reference/README.md): `decree check` passes here, `decree graph` reproduces `.decree/graph/*.md` byte for byte, and the recorded run replays through decree to the same events. [`feature`](../feature/README.md) walks through every other building block.

## Running it

Read it rather than run it. These commands only read the project:

```bash
cd examples/sort-documents
decree check                             # every machine and message is valid
decree graph                             # rewrites .decree/graph/ with no change
decree status 20261001T170412Z-3f9a51    # the run: its states, decisions and logs
```

In a real project, a script that finds a new scan queues it with `decree emit --machine sort_document --param file=scans/<name>.pdf`.

## The files

```text
examples/sort-documents/.decree/
  machines/
    sort_document.yml               the ladder: two checks, a local model, a large model, a person
    local_router.yml                a router for a small local classifier that scores every option
    router.yml                      the default router: asks Claude (the same file `decree init` writes)
  scripts/
    sort_document/extract_text.sh   prints the scan's text
    sort_document/file_away.sh      moves the scan to filed/<kind>/
    local_router/ask_local.sh       asks the classifier service
    router/ask_claude.sh            asks Claude
    ask_person.sh                   tells a person the options and how to reply
  runs/                             the recorded run and its two router child runs
  graph/  schema/                   written by `decree graph` and `decree schema`
```

## The ladder

[`machines/sort_document.yml`](.decree/machines/sort_document.yml) files one scanned document. It tries the cheapest way to decide first, and each step hands on only what it could not decide:

| Step | State | How it decides | Acts when | Otherwise |
| --- | --- | --- | --- | --- |
| 1 | `by_name` | `check: { data: file, matches: '^scans/invoice-[0-9]+\.pdf$' }` (free, certain) | `true` | `false` |
| 2 | `by_text` | `check: { output: read_text, matches: '(?i)invoice (no\|number)[.:]' }` (free, likely) | `true` | `false` |
| 3 | `local_model` | `model: { router: local_router, min_confidence: 0.9, output: read_text }` (a small CPU classifier, measured scores) | confidence ≥ 0.9 | `unsure` |
| 4 | `big_model` | `model: { min_confidence: 0.7, output: read_text }` (the default router, Claude) | confidence ≥ 0.7 | `unsure` |
| 5 | `worth_asking` | `check: { confidence: big_model, at_least: 0.4 }` | at least 0.4: `true`, ask a person | below 0.4: `false`, set aside |
| 6 | `ask_person` | `person: { ask: ask_person }` | the person's pick | no reply in a week: `error`, set aside |

So there are three thresholds, and each one is a number in the machine: 0.9 to trust the small model, 0.7 to trust the large one, and at least 0.4 to be worth a person's time. Each model's threshold is calibrated for that model: GLiNER2.5-Decide's scores are probabilities across the options, while Claude's confidence is self-reported. Where an option leads is written once per deciding state, so each state's options are exactly what that model or person sees.

A `model` state never calls a model itself: it hands the question to a router machine, which runs as a child run with its own folder. `local_model` names `router: local_router`; `big_model` names none, so it uses `router`. [`docs/routers.md`](../../docs/routers.md) shows how to write one.

## One run, every step

[`runs/20261001T170412Z-3f9a51/`](.decree/runs/20261001T170412Z-3f9a51/events.jsonl) climbs every step. The scan is `scans/2026-10-01-scan-0412.pdf`, an order confirmation marked PAID:

1. `by_name` said `false`, because the name is not `invoice-<n>.pdf` (event 2). `read_text` printed the text ([log](.decree/runs/20261001T170412Z-3f9a51/0001-read_text-extract_text.log)), and `by_text` found no invoice number (event 6).
2. `local_model` ran `local_router` as child run [`20261001T170412Z-b72e06`](.decree/runs/20261001T170412Z-b72e06/0001-ask-ask_local.log). The classifier scored receipt 0.62, invoice 0.31 and other 0.07 ([`reply.json`](.decree/runs/20261001T170412Z-b72e06/reply.json)). 0.62 is below 0.9, so the event was `unsure` (event 9), and the `decision` event keeps the pick and all three scores.
3. `big_model` asked Claude through `router` ([`20261001T170412Z-d10c3a`](.decree/runs/20261001T170412Z-d10c3a/0001-ask-ask_claude.log)), with the run so far in the request's `history`. Claude also picked receipt, but only at 0.55: "titled as an order, not a receipt". That is below 0.7, so `unsure` again (event 12).
4. `worth_asking` checked that 0.55 is at least 0.4: `true` (event 14). Had Claude said 0.2, the run would have ended in `set_aside`, and nobody would have been asked.
5. `ask_person` printed how to reply ([log](.decree/runs/20261001T170412Z-3f9a51/0002-ask_person-ask_person.log)) and the run waited on `20261001T170412Z-3f9a51.w15`. Eighteen minutes later a reply arrived ([`received/`](.decree/runs/20261001T170412Z-3f9a51/received/20261001T172208Z-e7d204.md)) with `event: receipt`, and `file_away` moved the scan to `filed/receipt/`.

Most scans would stop at step 1 or 2 and cost nothing; few would ever reach a person.
