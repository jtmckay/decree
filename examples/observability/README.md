# Observability: decree runs in Loki, Grafana and Jaeger

decree adds no metrics endpoint and no exporter: `events.jsonl` is the telemetry as well as the record, and `traces.jsonl` holds the same run as OpenTelemetry spans ([Observability](../../docs/reference/observability.md)). This example ships the events, and optionally each script's output, to Loki with Grafana Alloy, and lists LogQL queries to explore them in Grafana; and it ships the traces to Jaeger with the OpenTelemetry Collector ([Traces](#traces)). It has no `.decree/` of its own: point it at the recorded runs in [`feature`](../feature/README.md) or [`sort-documents`](../sort-documents/README.md), or at your own project.

## What gets shipped

[`config.alloy`](config.alloy) tails two kinds of file under `/srv/project/.decree/runs/`:

- `*/events.jsonl`, one JSON line per event, as `job="decree"`. Every line is self-contained (`run_id`, `machine`, `trigger`, `type`, `ts`), and `script` and `router` events carry `started_at` and `duration_ms`.
- `*/*.log`, each script's output, as `job="decree_script"`. The filename carries its context: `runs/<run_id>/<NNNN>-<state>-<script>.log`. State and script names cannot contain `-`, so the path regex is unambiguous.

Labels stay low-cardinality: `job`, `machine`, `type`, and `script` for script output. `run_id` (and `state` for script output) goes in structured metadata; a label per run makes Loki slow. Timestamps come from each event's `ts`, so late or back-filled lines land at the right time.

## Running it

Run Loki, Grafana and Alloy on one Docker network, with an example project mounted where `config.alloy` looks for it:

```bash
cd examples/observability
docker network create decree-observability
docker run -d --name loki --network decree-observability -p 3100:3100 grafana/loki -config.file=/etc/loki/local-config.yaml -validation.reject-old-samples=false
docker run -d --name grafana --network decree-observability -p 3000:3000 -e GF_AUTH_ANONYMOUS_ENABLED=true -e GF_AUTH_ANONYMOUS_ORG_ROLE=Admin grafana/grafana
docker run -d --name alloy --network decree-observability -v "$PWD/config.alloy:/etc/alloy/config.alloy:ro" -v "$PWD/../feature:/srv/project:ro" grafana/alloy run /etc/alloy/config.alloy
```

- `-validation.reject-old-samples=false` lets Loki take the recorded runs, whose `ts` are older than Loki's default limit of a week. A live project does not need it.
- Mount `../sort-documents` instead of `../feature` to ship that example's run, or your own project's root to ship yours. On a host without Docker, run `alloy run config.alloy` with `/srv/project` replaced by the directory that contains `.decree/`.
- Open Grafana at http://localhost:3000, add a Loki data source with the URL `http://loki:3100`, and run the queries below in Explore. The recorded runs are from 2026-10-01, so set the time range to include that day.

## Queries

```logql
# p95 script duration by script, last 24 h
quantile_over_time(0.95, {job="decree", type="script"} | json | unwrap duration_ms [24h]) by (script)

# runs that ended in failed, per machine, per day
sum by (machine) (count_over_time({job="decree", type="run_finished"} | json | state="failed" [1d]))

# interrupted runs waiting for `decree retry`
{job="decree", type="interrupted"} | json

# model decisions below a confidence floor
{job="decree", type="decision"} | json | kind="model" and confidence < 0.6

# the slowest agent rounds this week
topk(10, max_over_time({job="decree", type="script", machine="feature"} | json | script="implement" | unwrap duration_ms [7d]) by (run_id))

# one run's script output, in order
{job="decree_script"} | run_id="01-rate-limit-upload"
```

`state`, `run_id` and `machine` on every event are also what a UI built outside decree needs to link each node of a [graph](../../docs/reference/graph.md) to its logs here.

## Traces

[`otel-collector.yaml`](otel-collector.yaml) is an OpenTelemetry Collector configuration: its `otlp_json_file` receiver reads every `/srv/project/.decree/runs/*/traces.jsonl`, one OTLP/JSON request per line, and its `otlp_http` exporter sends the spans to an OTLP endpoint, here Jaeger ([Traces](../../docs/reference/observability.md#traces)). Run Jaeger and the Collector (contrib distribution, which has the receiver) on the same Docker network:

```bash
docker run -d --name jaeger --network decree-observability -p 16686:16686 jaegertracing/jaeger
docker run -d --name otelcol --network decree-observability -v "$PWD/otel-collector.yaml:/etc/otelcol-contrib/config.yaml:ro" -v "$PWD/../feature:/srv/project:ro" otel/opentelemetry-collector-contrib
```

Open Jaeger at http://localhost:16686, pick the service `decree`, and set the time range to include 2026-10-01. Each recorded run is one trace: `run feature` holds its scripts and decisions, and the router run sits under the `decision model triage` span that started it. A script that calls an instrumented service or model with the `TRACEPARENT` decree gives it adds its own spans under its `script` span.
