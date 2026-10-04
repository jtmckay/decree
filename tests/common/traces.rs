//! `runs/<id>/traces.jsonl` (docs/reference/observability.md, Traces) checked two ways: each
//! line has the OTLP/JSON shape of an `ExportTraceServiceRequest` (resourceSpans, scopeSpans,
//! spans; hex ids of the right length; Unix nanosecond times as decimal strings; enums as
//! integers), per <https://opentelemetry.io/docs/specs/otlp/#json-protobuf-encoding>; and its
//! spans agree with the run's `events.jsonl`: one span per `span_id` an event names that has
//! ended, with the events' own times. The shape is checked here directly, without a schema:
//! decree publishes none for a format OpenTelemetry defines. Used by `trace_test.rs` and
//! `schema_test.rs` through `#[path]`.

// Each including test file uses only some of these.
#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use chrono::{DateTime, Utc};
use serde_json::Value;

fn is_hex(v: &Value, len: usize) -> bool {
    v.as_str().is_some_and(|s| {
        s.len() == len
            && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
            && s.bytes().any(|b| b != b'0')
    })
}

fn nanos_of(v: &Value) -> u128 {
    let s = v
        .as_str()
        .unwrap_or_else(|| panic!("time {v} is not a string"));
    assert!(s.bytes().all(|b| b.is_ascii_digit()), "time {s}");
    s.parse().unwrap()
}

/// Unix nanoseconds of an RFC 3339 event timestamp, as decree writes them.
pub fn nanos(ts: &str) -> u128 {
    let t: DateTime<Utc> = DateTime::parse_from_rfc3339(ts).unwrap().into();
    t.timestamp_nanos_opt().unwrap() as u128
}

/// The lines of a JSON Lines file, parsed; a missing file has none.
pub fn json_lines(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{}: {e}", path.display())))
        .collect()
}

/// Every span in `run_dir/traces.jsonl`, after checking that each line is one OTLP/JSON
/// `ExportTraceServiceRequest` holding one resource (`service.name` `decree` and a
/// `service.version`), one `decree` scope and one span.
pub fn spans(run_dir: &Path) -> Vec<Value> {
    let path = run_dir.join("traces.jsonl");
    json_lines(&path)
        .into_iter()
        .map(|line| {
            let at = path.display();
            let rs = line["resourceSpans"].as_array().unwrap();
            assert_eq!(rs.len(), 1, "{at}: {line}");
            let attrs: BTreeMap<&str, &Value> = rs[0]["resource"]["attributes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|kv| (kv["key"].as_str().unwrap(), &kv["value"]))
                .collect();
            assert_eq!(attrs["service.name"]["stringValue"], "decree", "{at}");
            assert!(attrs["service.version"]["stringValue"].is_string(), "{at}");
            let ss = rs[0]["scopeSpans"].as_array().unwrap();
            assert_eq!(ss.len(), 1, "{at}: {line}");
            assert_eq!(ss[0]["scope"]["name"], "decree", "{at}");
            let spans = ss[0]["spans"].as_array().unwrap();
            assert_eq!(spans.len(), 1, "{at}: {line}");
            let span = spans[0].clone();
            assert!(is_hex(&span["traceId"], 32), "{at}: {span}");
            assert!(is_hex(&span["spanId"], 16), "{at}: {span}");
            if let Some(parent) = span.get("parentSpanId") {
                assert!(is_hex(parent, 16), "{at}: {span}");
            }
            assert!(span["name"].is_string(), "{at}: {span}");
            assert!(span["kind"].is_u64(), "{at}: enums are integers: {span}");
            assert!(
                nanos_of(&span["startTimeUnixNano"]) <= nanos_of(&span["endTimeUnixNano"]),
                "{at}: {span}"
            );
            for kv in span["attributes"].as_array().unwrap() {
                assert!(kv["key"].is_string(), "{at}: {kv}");
                let v = &kv["value"];
                let ok = v["stringValue"].is_string()
                    || v["intValue"]
                        .as_str()
                        .is_some_and(|s| s.parse::<i64>().is_ok());
                assert!(ok, "{at}: {kv}");
            }
            if let Some(status) = span.get("status") {
                assert!(status["code"].is_u64(), "{at}: {span}");
            }
            span
        })
        .collect()
}

/// A span's attribute, as a string (an `intValue` is one already).
pub fn attr<'s>(span: &'s Value, key: &str) -> Option<&'s str> {
    span["attributes"]
        .as_array()?
        .iter()
        .find(|kv| kv["key"] == key)
        .and_then(|kv| {
            kv["value"]["stringValue"]
                .as_str()
                .or(kv["value"]["intValue"].as_str())
        })
}

/// Whether `span` has status `ERROR` (trace.proto `STATUS_CODE_ERROR`, 2).
pub fn is_error(span: &Value) -> bool {
    span["status"]["code"] == 2
}

/// Check `run_dir`'s spans against its events, and return them by span id: every event
/// carries the run's one `trace_id`; span ids are unique; and there is exactly one span for
/// each ended span an event names, with its start and end at the events' times: a `script`
/// (`started_at` to `started_at` + `duration_ms`), a `decision` (its `ts`, from the `waiting`
/// for a router run), a `received` wait (from the `waiting`), and the run, from the claim or
/// `retry` that started it to the `run_finished` or `interrupted` that ended it.
pub fn agree_with_events(run_dir: &Path) -> BTreeMap<String, Value> {
    let events = json_lines(&run_dir.join("events.jsonl"));
    let at = run_dir.display();
    let trace_ids: BTreeSet<&str> = events
        .iter()
        .map(|e| e["trace_id"].as_str().unwrap())
        .collect();
    assert_eq!(trace_ids.len(), 1, "{at}: {trace_ids:?}");
    let trace_id = *trace_ids.iter().next().unwrap();

    let mut by_id: BTreeMap<String, Value> = BTreeMap::new();
    for span in spans(run_dir) {
        assert_eq!(span["traceId"], trace_id, "{at}: {span}");
        let id = span["spanId"].as_str().unwrap().to_string();
        assert!(
            by_id.insert(id.clone(), span).is_none(),
            "{at}: span {id} twice"
        );
    }
    let mut expected: BTreeMap<String, (u128, u128)> = BTreeMap::new();
    let mut waiting: Option<&Value> = None;
    let mut run: Option<(String, u128)> = None;
    for e in &events {
        let ts = || nanos(e["ts"].as_str().unwrap());
        let span = e["span_id"].as_str().map(String::from);
        match e["type"].as_str().unwrap() {
            "transition" if span.is_some() => run = Some((span.unwrap(), ts())),
            "waiting" => waiting = Some(e),
            "script" => {
                let start = nanos(e["started_at"].as_str().unwrap());
                let end = start + e["duration_ms"].as_u64().unwrap() as u128 * 1_000_000;
                expected.insert(span.unwrap(), (start, end));
            }
            "decision" => {
                let router = waiting
                    .filter(|w| e.get("child_run").is_some() && w["child"] == e["child_run"]);
                let start = router.map_or_else(ts, |w| nanos(w["ts"].as_str().unwrap()));
                expected.insert(span.unwrap(), (start, ts()));
            }
            "received" => {
                let w = waiting.expect("received without waiting");
                expected.insert(span.unwrap(), (nanos(w["ts"].as_str().unwrap()), ts()));
            }
            "run_finished" | "interrupted" => {
                let (id, start) = run.clone().expect("a run span");
                expected.insert(id, (start, ts()));
            }
            _ => {}
        }
    }
    let ids: BTreeSet<&String> = by_id.keys().collect();
    let want: BTreeSet<&String> = expected.keys().collect();
    assert_eq!(ids, want, "{at}: spans vs the events' span ids");
    for (id, (start, end)) in &expected {
        let span = &by_id[id];
        assert_eq!(
            nanos_of(&span["startTimeUnixNano"]),
            *start,
            "{at}: start of {span}"
        );
        assert_eq!(
            nanos_of(&span["endTimeUnixNano"]),
            *end,
            "{at}: end of {span}"
        );
    }
    by_id
}
