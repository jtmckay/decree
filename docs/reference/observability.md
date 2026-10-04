# Observability

`events.jsonl` is the telemetry interface: decree adds no metrics endpoint and no exporter. Any log shipper that tails files can read it; the reference setup is Grafana Alloy into Loki, shown in [`mock/observability/config.alloy`](../../mock/observability/config.alloy).

- **Ship** `.decree/runs/*/events.jsonl` (events) and, optionally, `.decree/runs/*/*.log` (script output).
- **Timestamps.** Use the event's `ts` as the log timestamp, so back-filled and late-shipped events land at the right time.
- **Labels** must stay low-cardinality: `machine`, `type`, and for script output `script`. `run_id`, `state` and `seq` are fields or structured metadata, never labels, because a label per run makes Loki slow.
- **Script output files** carry their context in the path: `runs/<run_id>/<NNNN>-<state>-<script>.log`. State and script names cannot contain `-`, so the regex `/runs/(?P<run_id>[^/]+)/(?P<n>\d{4,})-(?P<state>[^-]+)-(?P<script>[^/]+)\.log$` is unambiguous.
- **Retention.** Loki's retention is the history. `decree prune --older-than <age>` deletes finished run folders ([cli.md](cli.md)), and nothing else does, so the local `runs/` is a working copy: ship runs before pruning them, and prune with an age longer than the shipper's lag.
- **Stability.** Field names and meanings in [events.jsonl](runs.md#eventsjsonl) are a public contract under `v: 1`. Dashboards may depend on them.

Example LogQL, also in [`mock/README.md`](../../mock/README.md):

```logql
# p95 script duration by script, last 24 h
quantile_over_time(0.95, {job="decree", type="script"} | json | unwrap duration_ms [24h]) by (script)

# runs that ended in failed, per machine, per day
sum by (machine) (count_over_time({job="decree", type="run_finished"} | json | state="failed" [1d]))

# interrupted runs waiting for `decree retry`
{job="decree", type="interrupted"} | json

# model decisions below a confidence floor
{job="decree", type="decision"} | json | kind="model" and confidence < 0.6
```

The events map directly onto OpenTelemetry traces (run = trace, script and decision events = spans with start and duration), so an OTLP exporter could be added without changing the schema. decree has none.
