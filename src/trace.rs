//! Traces (docs/reference/observability.md, Traces): W3C Trace Context ids for every run, the
//! `TRACEPARENT` and `TRACESTATE` a script gets, and `runs/<id>/traces.jsonl`, where each
//! finished span is appended as one OTLP/JSON `ExportTraceServiceRequest`.
//!
//! - W3C Trace Context, <https://www.w3.org/TR/trace-context/>: `traceparent` is
//!   `<version>-<trace-id>-<parent-id>-<trace-flags>`, lowercase hex; an all-zero trace id or
//!   parent id is invalid, and `tracestate` is only read beside a valid `traceparent`.
//! - OpenTelemetry environment variable carriers,
//!   <https://opentelemetry.io/docs/specs/otel/context/env-carriers/>: the `traceparent` key
//!   normalised to an environment variable name is `TRACEPARENT`, `tracestate` `TRACESTATE`.
//! - OTLP/JSON, <https://opentelemetry.io/docs/specs/otlp/#json-protobuf-encoding>: field
//!   names in lowerCamelCase, trace and span ids as hex strings, enums as integers, 64-bit
//!   integers (times in Unix nanoseconds, `intValue`) as decimal strings. The fields are those
//!   of `opentelemetry/proto/trace/v1/trace.proto`. The OpenTelemetry Collector's
//!   `otlp_json_file` receiver reads such files, one request per line, as the OTLP file
//!   exporter writes them (<https://opentelemetry.io/docs/specs/otel/protocol/file-exporter/>).

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::Path;

use chrono::{DateTime, Utc};
use serde_json::{json, Value};

/// The run's spans, in the run directory (docs/reference/observability.md, Traces).
pub const TRACES_FILE: &str = "traces.jsonl";

/// The frontmatter keys and environment variables that carry the context.
pub const TRACEPARENT_KEY: &str = "traceparent";
pub const TRACESTATE_KEY: &str = "tracestate";
pub const TRACEPARENT_ENV: &str = "TRACEPARENT";
pub const TRACESTATE_ENV: &str = "TRACESTATE";

/// The `traceparent` version decree reads and writes.
const VERSION: &str = "00";

/// Trace flags decree writes: sampled, since every span is recorded.
const SAMPLED: &str = "01";

/// A valid W3C `traceparent`: the trace id and the parent span id, in lowercase hex.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceParent {
    pub trace_id: String,
    pub parent_id: String,
}

impl TraceParent {
    /// Parse a version `00` `traceparent`: exactly 55 characters, `00-<32 hex>-<16 hex>-<2
    /// hex>`, lowercase, with neither id all zeros. Anything else is `None`, and the caller
    /// starts a new trace (W3C Trace Context, traceparent header).
    pub fn parse(text: &str) -> Option<TraceParent> {
        let mut parts = text.split('-');
        let (version, trace_id, parent_id, flags) =
            (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
        let valid = parts.next().is_none()
            && version == VERSION
            && is_id(trace_id, 32)
            && is_id(parent_id, 16)
            && is_hex(flags, 2);
        valid.then(|| TraceParent {
            trace_id: trace_id.to_string(),
            parent_id: parent_id.to_string(),
        })
    }

    /// `00-<trace id>-<span id>-01`.
    pub fn format(trace_id: &str, span_id: &str) -> String {
        format!("{VERSION}-{trace_id}-{span_id}-{SAMPLED}")
    }
}

/// `len` lowercase hex digits.
fn is_hex(text: &str, len: usize) -> bool {
    text.len() == len && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// A trace or span id: `len` lowercase hex digits, not all zeros.
pub fn is_id(text: &str, len: usize) -> bool {
    is_hex(text, len) && text.bytes().any(|b| b != b'0')
}

/// A `tracestate` decree passes on unchanged: at most 512 printable ASCII characters
/// (W3C Trace Context asks vendors to propagate at least 512) and at most 32 list-members.
/// decree never parses its members.
pub fn valid_tracestate(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 512
        && text.bytes().all(|b| (0x20..=0x7e).contains(&b))
        && text.split(',').count() <= 32
}

/// The context a message carries in its frontmatter: a valid `traceparent`, and the
/// `tracestate` beside it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Incoming {
    pub parent: Option<TraceParent>,
    pub tracestate: Option<String>,
}

impl Incoming {
    /// From frontmatter values. An invalid `traceparent` is ignored, and so is any
    /// `tracestate` then, since W3C Trace Context reads `tracestate` only beside a valid
    /// `traceparent`.
    pub fn new(traceparent: Option<&str>, tracestate: Option<&str>) -> Incoming {
        let parent = traceparent.and_then(TraceParent::parse);
        let tracestate = tracestate
            .filter(|_| parent.is_some())
            .filter(|s| valid_tracestate(s))
            .map(String::from);
        Incoming { parent, tracestate }
    }
}

/// `bytes` random bytes from `/dev/urandom`, as lowercase hex; never all zeros.
fn random_hex(bytes: usize) -> String {
    loop {
        let mut buf = vec![0u8; bytes];
        let read = File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut buf));
        if let Err(e) = read {
            // No randomness means no unique ids; nothing sensible can go on.
            panic!("cannot read /dev/urandom: {e}");
        }
        if buf.iter().any(|&b| b != 0) {
            return buf.iter().map(|b| format!("{b:02x}")).collect();
        }
    }
}

/// A new random 16-byte trace id (W3C Trace Context, trace-id).
pub fn new_trace_id() -> String {
    random_hex(16)
}

/// A new random 8-byte span id (W3C Trace Context, parent-id).
pub fn new_span_id() -> String {
    random_hex(8)
}

/// An attribute value: OTLP `AnyValue`.
#[derive(Debug, Clone, PartialEq)]
pub enum Attr {
    Str(String),
    Int(i64),
}

/// One finished span.
#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    pub trace_id: String,
    pub span_id: String,
    /// Empty for a root span (trace.proto, `parent_span_id`).
    pub parent_span_id: Option<String>,
    pub name: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub attributes: Vec<(&'static str, Attr)>,
    /// Spans in the same trace this one follows: a retried run's previous run span.
    pub links: Vec<String>,
    /// `Some(message)` gives status `ERROR` with that message; `None` leaves it unset.
    pub error: Option<String>,
}

/// trace.proto `SpanKind`: `SPAN_KIND_INTERNAL`.
const SPAN_KIND_INTERNAL: u8 = 1;

/// trace.proto `Status.StatusCode`: `STATUS_CODE_ERROR`.
const STATUS_CODE_ERROR: u8 = 2;

/// Unix nanoseconds as a decimal string, as OTLP/JSON writes a `fixed64`.
fn nanos(t: DateTime<Utc>) -> String {
    t.timestamp_nanos_opt().unwrap_or_default().to_string()
}

fn key_value(key: &str, value: &Attr) -> Value {
    let value = match value {
        Attr::Str(s) => json!({ "stringValue": s }),
        // int64 is a decimal string in OTLP/JSON.
        Attr::Int(n) => json!({ "intValue": n.to_string() }),
    };
    json!({ "key": key, "value": value })
}

impl Span {
    /// The span as one OTLP/JSON `ExportTraceServiceRequest`: one resource (`service.name`
    /// `decree`, `service.version`, from OpenTelemetry's semantic conventions), one scope
    /// (`decree`), one span.
    pub fn to_otlp(&self) -> Value {
        let mut span = json!({
            "traceId": self.trace_id,
            "spanId": self.span_id,
            "name": self.name,
            "kind": SPAN_KIND_INTERNAL,
            "startTimeUnixNano": nanos(self.start),
            "endTimeUnixNano": nanos(self.end),
            "attributes": self
                .attributes
                .iter()
                .map(|(k, v)| key_value(k, v))
                .collect::<Vec<_>>(),
        });
        if let Some(parent) = &self.parent_span_id {
            span["parentSpanId"] = json!(parent);
        }
        if !self.links.is_empty() {
            span["links"] = self
                .links
                .iter()
                .map(|id| json!({ "traceId": self.trace_id, "spanId": id }))
                .collect();
        }
        if let Some(message) = &self.error {
            span["status"] = json!({ "code": STATUS_CODE_ERROR, "message": message });
        }
        let version = env!("CARGO_PKG_VERSION");
        json!({
            "resourceSpans": [{
                "resource": {
                    "attributes": [
                        key_value("service.name", &Attr::Str("decree".into())),
                        key_value("service.version", &Attr::Str(version.into())),
                    ],
                },
                "scopeSpans": [{
                    "scope": { "name": "decree", "version": version },
                    "spans": [span],
                }],
            }],
        })
    }
}

/// Append `span` to `run_dir/traces.jsonl` as one line, with a single write to a file
/// opened with `O_APPEND`, as `events.jsonl` is written.
pub fn append_span(run_dir: &Path, span: &Span) -> io::Result<()> {
    let mut line = serde_json::to_vec(&span.to_otlp()).map_err(io::Error::other)?;
    line.push(b'\n');
    OpenOptions::new()
        .append(true)
        .create(true)
        .open(run_dir.join(TRACES_FILE))?
        .write_all(&line)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";

    #[test]
    fn traceparent_parses_the_w3c_example() {
        let p = TraceParent::parse(VALID).unwrap();
        assert_eq!(p.trace_id, "4bf92f3577b34da6a3ce929d0e0e4736");
        assert_eq!(p.parent_id, "00f067aa0ba902b7");
        assert_eq!(
            TraceParent::format(&p.trace_id, &p.parent_id),
            VALID,
            "decree writes the sampled flag"
        );
    }

    #[test]
    fn traceparent_rejects_what_w3c_calls_invalid() {
        for bad in [
            "",
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7",
            "00-00000000000000000000000000000000-00f067aa0ba902b7-01",
            "00-4bf92f3577b34da6a3ce929d0e0e4736-0000000000000000-01",
            "00-4BF92F3577B34DA6A3CE929D0E0E4736-00f067aa0ba902b7-01",
            "ff-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
            "01-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
            "00-4bf92f3577b34da6a3ce929d0e0e473-00f067aa0ba902b7-01",
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01-00",
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-0g",
            " 00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
        ] {
            assert_eq!(TraceParent::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn tracestate_is_kept_only_beside_a_valid_traceparent() {
        let i = Incoming::new(Some(VALID), Some("congo=t61rcWkgMzE"));
        assert_eq!(i.tracestate.as_deref(), Some("congo=t61rcWkgMzE"));
        let i = Incoming::new(Some("00-bad"), Some("congo=t61rcWkgMzE"));
        assert_eq!(i, Incoming::default());
        let i = Incoming::new(Some(VALID), Some("bad\nvalue"));
        assert_eq!(i.tracestate, None);
        let too_many = vec!["a=b"; 33].join(",");
        assert!(!valid_tracestate(&too_many));
    }

    #[test]
    fn random_ids_are_hex_of_the_right_length_and_differ() {
        let (a, b) = (new_trace_id(), new_trace_id());
        assert!(is_id(&a, 32) && is_id(&b, 32) && a != b);
        assert!(is_id(&new_span_id(), 16));
    }

    #[test]
    fn otlp_json_has_the_encoding_rules() {
        let at = |ms: i64| DateTime::from_timestamp_millis(ms).unwrap();
        let span = Span {
            trace_id: "4bf92f3577b34da6a3ce929d0e0e4736".into(),
            span_id: "00f067aa0ba902b7".into(),
            parent_span_id: None,
            name: "script verify/verify".into(),
            start: at(1_790_000_000_123),
            end: at(1_790_000_001_000),
            attributes: vec![
                ("decree.state", Attr::Str("verify".into())),
                ("process.exit.code", Attr::Int(3)),
            ],
            links: vec!["1111111111111111".into()],
            error: Some("exit code 3".into()),
        };
        let v = span.to_otlp();
        let rs = &v["resourceSpans"][0];
        assert_eq!(
            rs["resource"]["attributes"][0],
            json!({"key": "service.name", "value": {"stringValue": "decree"}})
        );
        assert_eq!(rs["scopeSpans"][0]["scope"]["name"], "decree");
        let s = &rs["scopeSpans"][0]["spans"][0];
        assert_eq!(s["startTimeUnixNano"], "1790000000123000000");
        assert_eq!(s["endTimeUnixNano"], "1790000001000000000");
        assert_eq!(s["kind"], 1);
        assert_eq!(s["status"], json!({"code": 2, "message": "exit code 3"}));
        assert_eq!(s["attributes"][1]["value"], json!({"intValue": "3"}));
        assert_eq!(s["links"][0]["spanId"], "1111111111111111");
        // A root span has no parentSpanId.
        assert!(s.get("parentSpanId").is_none());
    }
}
