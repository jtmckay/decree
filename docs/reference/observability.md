# Observability

`events.jsonl` is the telemetry interface, with `traces.jsonl` beside it for OpenTelemetry tools ([Traces](#traces)): decree adds no metrics endpoint and no exporter. Any log shipper that tails files can read it; the reference setup is Grafana Alloy into Loki, shown in [`examples/observability/`](../../examples/observability/README.md).

- **Ship** `.decree/runs/*/events.jsonl` (events) and, optionally, `.decree/runs/*/*.log` (script output). `traces.jsonl` goes to a trace store instead ([Traces](#traces)).
- **Timestamps.** Use the event's `ts` as the log timestamp, so back-filled and late-shipped events land at the right time.
- **Labels** must stay low-cardinality: `machine`, `type`, and for script output `script`. `run_id`, `state` and `seq` are fields or structured metadata, never labels, because a label per run makes Loki slow.
- **Script output files** carry their context in the path: `runs/<run_id>/<NNNN>-<state>-<script>.log`. State and script names cannot contain `-`, so the regex `/runs/(?P<run_id>[^/]+)/(?P<n>\d{4,})-(?P<state>[^-]+)-(?P<script>[^/]+)\.log$` is unambiguous.
- **Retention.** Loki's retention is the history. `decree prune --older-than <age>` deletes finished run folders ([cli.md](cli.md)), and nothing else does, so the local `runs/` is a working copy: ship runs before pruning them, and prune with an age longer than the shipper's lag.
- **Stability.** Field names and meanings in [events.jsonl](runs.md#eventsjsonl) are a public contract under `v: 1`. Dashboards may depend on them.

Example LogQL, also in [`examples/observability/README.md`](../../examples/observability/README.md):

```logql
# p95 script duration by script, last 24 h
quantile_over_time(0.95, {job="decree", type="script"} | json | unwrap duration_ms [24h]) by (script)

# runs that ended in failed, per machine, per day
sum by (machine) (count_over_time({job="decree", type="run_finished"} | json | state="failed" [1d]))

# interrupted runs waiting for `decree process --retry`
{job="decree", type="interrupted"} | json

# model decisions below a confidence floor
{job="decree", type="decision"} | json | kind="model" and confidence < 0.6
```

## Traces

A run is also a trace: a tree of timed spans, into which child runs, router calls and the scripts' own calls to services and models nest. decree writes it to files in standard formats and passes its context to scripts in standard environment variables; it has no exporter and no network code. An OpenTelemetry Collector ships the files ([decisions.md](../decisions.md#d52-traces-as-files-and-environment-variables-not-an-exporter)).

Standards:

- **W3C Trace Context** ([w3.org/TR/trace-context](https://www.w3.org/TR/trace-context/)): a `traceparent` is `00-<trace id>-<parent span id>-<trace flags>`, 32, 16 and 2 lowercase hex digits; a trace id or span id of all zeros is invalid, and `tracestate` is only read beside a valid `traceparent`.
- **OpenTelemetry's environment variable carrier** ([Environment Variables as Context Propagation Carriers](https://opentelemetry.io/docs/specs/otel/context/env-carriers/)): the `traceparent` and `tracestate` keys, uppercased, are the environment variables `TRACEPARENT` and `TRACESTATE` a parent process sets for a child process.
- **OTLP/JSON** ([OTLP, JSON Protobuf Encoding](https://opentelemetry.io/docs/specs/otlp/#json-protobuf-encoding)): the JSON encoding of an OTLP `ExportTraceServiceRequest`, with the fields of [`trace.proto`](https://github.com/open-telemetry/opentelemetry-proto/blob/main/opentelemetry/proto/trace/v1/trace.proto). Field names are lowerCamelCase, trace and span ids hex strings, enums integers, and 64-bit integers (times, `intValue`) decimal strings. One request per line is the [OTLP file exporter](https://opentelemetry.io/docs/specs/otel/protocol/file-exporter/) format, which the Collector's [`otlp_json_file` receiver](https://github.com/open-telemetry/opentelemetry-collector-contrib/tree/main/receiver/otlpjsonfilereceiver) reads.

**Ids.** A run takes its trace id from its message's `traceparent` frontmatter key if it is valid, and its run span's parent is the span `traceparent` names; otherwise the run starts a new trace with a random trace id and its run span is a root. Trace ids (16 bytes) and span ids (8 bytes) are read from `/dev/urandom`. Every event in `events.jsonl` carries the run's `trace_id`, and each event that starts or ends a span carries its `span_id` ([events.jsonl](runs.md#eventsjsonl)), so the record and the trace name the same spans.

**Context passed on.**

- **Scripts** get `TRACEPARENT=00-<trace id>-<the script's span id>-01`, and `TRACESTATE` when the message carried one ([Environment](scripts.md#environment)). An instrumented call a script makes (an OpenTelemetry SDK, or a CLI that reads `TRACEPARENT`) becomes a child of the script's span.
- **Child runs** get `traceparent` in their `message.md`, naming the invoking state's span: the router's `decision` span for a `model` invoke, the wait span for a `machine` invoke ([Sub-machines](runs.md#sub-machines)).
- **`decree emit`** run by a script sets `traceparent` (and `tracestate`) on the new message from `TRACEPARENT` (and `TRACESTATE`), so follow-up work joins the trace under the emitting script.
- **Any inbox message** may carry `traceparent` from outside, for example from a webhook relaying an upstream trace ([Frontmatter keys](messages.md#frontmatter-keys)). An invalid one is ignored, and the run starts a new trace.

**Spans.** Every span's start and end are the times of the events that bound it, so the trace and `events.jsonl` agree:

| Span | Name | Parent | From | To |
| --- | --- | --- | --- | --- |
| The run | `run <machine>` | The span the message's `traceparent` names, else none | The claim event, or a `retry` event | `run_finished` or `interrupted` |
| A script execution (`onentry`, invoke attempt, `onexit`) | `script <state>/<script>` | The run span | `started_at` of its `script` event | `started_at` + `duration_ms` |
| A decision (`check`, `model`, `person`) | `decision <kind> <state>` | The run span | Its `decision` event; for a router run, the `waiting` event for it | Its `decision` event |
| A wait (a `person` reply, or a `machine` invoke's child run) | `wait <state>` | The run span | The `waiting` event | The `received` event |

A router run's run span is a child of its `model` decision's span, and a `machine` invoke's child run's run span is a child of its wait span. A run continued by `decree process --retry` gets a new run span in the same trace, from the `retry` event, linked to the previous run span. Every span has the attributes `decree.run_id`, `decree.machine` and `decree.state`; script spans add `decree.attempt` and `process.exit.code` (from [OpenTelemetry's semantic conventions](https://opentelemetry.io/docs/specs/semconv/registry/attributes/process/), when the script exited rather than being killed), and decision and wait spans add `decree.event`. A script that exits non-zero, times out or is killed, a `model` decision with a `router_error`, a run ending in `failed`, and an interrupted run have status `ERROR`. All spans are of kind `INTERNAL`.

**The file.** `runs/<id>/traces.jsonl` holds the run's spans, each appended when it ends as one OTLP/JSON `ExportTraceServiceRequest` line, with a single write on a file opened with `O_APPEND`, as `events.jsonl` is written. Each line has one resource (`service.name: decree` and `service.version`, from the [semantic conventions](https://opentelemetry.io/docs/specs/semconv/registry/attributes/service/)), one scope (`decree`) and one span. A script span of the recorded `feature` migration, its keys in `trace.proto` order (decree writes them in alphabetical order, which a JSON reader ignores):

```json
{"resourceSpans":[{"resource":{"attributes":[{"key":"service.name","value":{"stringValue":"decree"}},{"key":"service.version","value":{"stringValue":"0.5.0"}}]},"scopeSpans":[{"scope":{"name":"decree","version":"0.5.0"},"spans":[{"traceId":"94b30376f6a9be8a642b186df56c40ec","spanId":"e496ca7dbf9e719c","parentSpanId":"bbd03ab4bd8e2e6c","name":"script verify/verify","kind":1,"startTimeUnixNano":"1790866179800000000","endTimeUnixNano":"1790866271900000000","attributes":[{"key":"decree.run_id","value":{"stringValue":"01-rate-limit-upload"}},{"key":"decree.machine","value":{"stringValue":"feature"}},{"key":"decree.state","value":{"stringValue":"verify"}},{"key":"decree.attempt","value":{"intValue":"1"}},{"key":"process.exit.code","value":{"intValue":"0"}}]}]}]}]}
```

A span still open when the process is killed is not written. The run span of a run interrupted by a signal is written with the `interrupted` event; after a crash, the recovery that appends `interrupted` (`process` or `daemon` starting) writes it, ended at that event, with status `ERROR`. A run waiting for a reply has written its finished spans; its run span is written when it finishes.

**Shipping.** The OpenTelemetry Collector (contrib distribution) reads the files with `otlp_json_file` and exports them to any OTLP endpoint (Jaeger, Grafana Tempo, Honeycomb, a vendor's agent). A minimal configuration, also in [`examples/observability/otel-collector.yaml`](../../examples/observability/otel-collector.yaml):

```yaml
receivers:
  otlp_json_file:
    include:
      - /srv/project/.decree/runs/*/traces.jsonl
    start_at: beginning
exporters:
  otlp_http:
    endpoint: http://localhost:4318
service:
  pipelines:
    traces:
      receivers: [otlp_json_file]
      exporters: [otlp_http]
```

Without a `storage` extension the receiver keeps its file offsets in memory, so a restarted Collector sends the files again from the beginning; add the `file_storage` extension and `storage: file_storage` to keep them. As with events, ship runs before `decree prune` removes them.
