---
routine: rust-develop
---

# 26: decree cron list command

## Overview

Add a `decree cron list` command that shows all cron files with their
schedule, last-run age, and countdown to next fire.

## Requirements

Add a `CronList` subcommand to the CLI:

```
decree cron list
```

Output format:

```
CRON FILE                       SCHEDULE           ROUTINE         LAST RUN   NEXT RUN
gmail-sync.md                   */15 * * * *       gmail-sync      2m ago     13m
actual-budget.md                */5  * * * *       actual-budget   1m ago     4m
notes.md                        */10 * * * *       notes           8m ago     2m
clean-runs.md                   0    * * * *       clean-runs      47m ago    13m
```

**LAST RUN**: scan `.decree/runs/` for directories whose name contains the
cron file's stem (e.g., stem `gmail-sync` matches run `D0001-1432-gmail-sync-0`).
Pick the most recent (alphabetically last, since names are sortable by date).
Display as relative age: `2m ago`, `47m ago`, `3h ago`. Show `never` if no
matching run exists.

**NEXT RUN**: compute from the cron expression using `Schedule::upcoming()`.
Display as a countdown: `13m`, `2m`, `3h`. Show `—` if the schedule cannot
be determined.

Columns are aligned with fixed-width padding. Sort rows by CRON FILE name.

Register the new command in `src/cli.rs` and implement in
`src/commands/cron_list.rs`.

## Files to Modify

- `src/cli.rs` — add `CronList` subcommand
- `src/main.rs` — dispatch `CronList`

## Files to Create

- `src/commands/cron_list.rs` — `decree cron list` implementation

## Acceptance Criteria

- **Given** cron files in `.decree/cron/` and matching run directories
  **When** `decree cron list` runs
  **Then** each cron file is listed with its schedule, inferred routine,
  relative last-run age, and countdown to next fire

- **Given** a cron file with no matching runs
  **When** `decree cron list` runs
  **Then** LAST RUN shows `never`

- **Given** no cron files exist
  **When** `decree cron list` runs
  **Then** an empty table or a "No cron files." message is printed and the
  command exits 0
