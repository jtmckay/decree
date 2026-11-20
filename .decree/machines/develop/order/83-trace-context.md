---
machine: rust_develop
---
# 83: W3C Trace Context and OpenTelemetry spans

## Overview

Enterprises trace work with OpenTelemetry. A decree run is naturally a trace: it is a tree of timed steps, and child runs, router calls and the scripts' own calls to services and models nest inside it. Decided:

- decree writes the trace in standard formats, with no network code and no new dependencies;
- it passes the context on to scripts and to the messages they emit, so their own instrumented calls join the same trace.

**Standards** (cite each in the docs):

- **W3C Trace Context.** `traceparent` is `00-<32 hex trace id>-<16 hex parent span id>-<2 hex flags>`.
- **OpenTelemetry's environment variable carrier.** `TRACEPARENT`, and `TRACESTATE` when one is known, are passed to child processes.
- **OTLP/JSON**, the JSON encoding of OTLP `ExportTraceServiceRequest`. The OpenTelemetry Collector's `otlpjsonfile` receiver reads it from files, one request per line.

## Requirements

Read `docs/reference/runs.md` (Step loop, Sub-machines, `events.jsonl`), `docs/reference/scripts.md` (Environment), `docs/reference/messages.md` (Frontmatter keys) and `docs/reference/observability.md` first. Before writing the OTLP/JSON encoder, fetch and read:

- the OTLP/JSON encoding rules: ids as hex strings, times as nanosecond strings, enum values;
- the `otlpjsonfile` receiver's README.

Cite both.

1. **Spans.**
   - **The run:** one span, from claim to `run_finished` or interruption. A run interrupted and continued by `decree retry` gets a new run span in the same trace, linked to the previous one.
   - **Each script execution:** a span (onentry, invoke attempt or onexit), a child of the run span.
   - **Each decision:** a span (`check`, `model`, `person`). A `model` decision's span is the parent of its router child run's run span, and a `machine` invoke's span is the parent of the child run's run span.
   - **A `person` wait:** spans from `waiting` to `received`.
   - **Names:** `run <machine>`, `script <state>/<script>`, `decision <kind> <state>`.
   - **Attributes:** `decree.run_id`, `decree.machine`, `decree.state`, `decree.event` and `decree.attempt`, plus `process.exit.code` for scripts (OpenTelemetry semantic conventions where one exists). A failed script or a run ending in `failed` gets status `ERROR`.
2. **Ids.**
   - **trace_id:** a run takes its trace id from its message's `traceparent` frontmatter key if valid, else a new random one. Read random bytes from `/dev/urandom`; no new dependency.
   - **span ids:** random.
   - **Child runs** get `traceparent` (the invoking decision's or state's span) in their `message.md`.
   - **`decree emit` from a script** sets `traceparent` on the new message from `TRACEPARENT`, so follow-up work joins the trace.
   - **Any inbox message** may carry `traceparent` from outside, for example a webhook relaying an upstream trace.
3. **Scripts** get `TRACEPARENT=00-<trace>-<the script's span id>-01`, and `TRACESTATE` when the message carried one, in their environment.
4. **`events.jsonl`:**
   - every event gains `trace_id`;
   - `script` and `decision` events gain `span_id`;
   - the claim `transition` gains `span_id` (the run span) and `parent_span_id` when there is a parent.
   
   These fields are additive, so `v` stays 1. Update `events.schema.json` and `docs/reference/runs.md`.
5. **`runs/<id>/traces.jsonl`:** each finished span is appended as one OTLP/JSON `ExportTraceServiceRequest` line, written with `O_APPEND` like `events.jsonl`. The `resource` attributes are `service.name: decree` and `service.version`. The scope is `decree`. Spans still open when the process is killed are not written; the next recovery writes the run span ended at the `interrupted` event, with status `ERROR`.
6. **Docs:**
   - `docs/reference/observability.md`: a "Traces" section explaining the ids, the env vars, the frontmatter key and the file, plus a minimal Collector config that reads `traces.jsonl` with `otlpjsonfile` and exports to an OTLP endpoint;
   - `docs/reference/messages.md`: the `traceparent` key;
   - `docs/reference/scripts.md`: `TRACEPARENT`;
   - `examples/observability/`: add the Collector config;
   - `CHANGELOG.md`;
   - `docs/decisions.md`: why files and env vars rather than an exporter (no network code, standard formats, the Collector does the shipping).
7. **Tests:**
   - one trace id across a run, its router child run and its machine child run, with correct parent span ids;
   - an inbox message's `traceparent` is honoured, and an invalid one is ignored, giving a new trace;
   - a script sees `TRACEPARENT` with its own span id;
   - `decree emit` from a script carries the trace;
   - every `traces.jsonl` line parses as JSON with the OTLP/JSON shape (resourceSpans, scopeSpans, spans, with hex ids of the right length and nanosecond time strings), and span start and end times match the events' times;
   - an interrupted run's span is written on recovery with `ERROR`;
   - the property test checks that every event has `trace_id`, and that every `span_id` is unique within a run.

- Only this migration's scope.
- If the reference docs and the code disagree, or a case is not covered here, do not guess: write the explanation to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- Tests build their own `.decree/` in a temp directory and never touch this repository's `.decree/`. No test calls a real LLM or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply, so it lands in the run log.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass.

## Acceptance Criteria

- **Given** a run with a router decision and a child machine
  **When** it finishes
  **Then** the run, the router run and the child run share one trace id, every span's parent is the documented one, and `traces.jsonl` has a span for each run, script and decision

- **Given** an inbox message with `traceparent: 00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01`
  **When** it runs
  **Then** its trace id is `4bf92f3577b34da6a3ce929d0e0e4736` and its run span's parent is `00f067aa0ba902b7`

- **Given** a script
  **When** it runs
  **Then** `TRACEPARENT` names the run's trace and the script's own span
