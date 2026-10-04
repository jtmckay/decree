//! `events.jsonl` (docs/reference/runs.md): the run's record. `EventLog` appends to it; the
//! functions below read it back and derive what the reference derives from it: the run's
//! state, visits and decision confidences.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::Path;

use chrono::{SecondsFormat, Utc};
use serde_json::{json, Map, Value};

/// One line of `events.jsonl`.
pub type Event = Map<String, Value>;

/// The run's event log, in the run directory (docs/reference/runs.md).
pub const EVENTS_FILE: &str = "events.jsonl";

/// `events.jsonl` schema version (docs/reference/runs.md).
const EVENTS_VERSION: u64 = 1;

/// RFC 3339 UTC with milliseconds, as every timestamp in `events.jsonl` is written.
pub fn timestamp(t: chrono::DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// `runs/<id>/events.jsonl`: one JSON object per line, each appended with a single write
/// to a file opened with `O_APPEND` (docs/reference/runs.md).
#[derive(Debug)]
pub struct EventLog {
    file: File,
    next_seq: u64,
    run_id: String,
    machine: String,
    trigger: String,
}

impl EventLog {
    /// Open or create the log in `run_dir`. `seq` continues after the lines already in it.
    pub fn open(run_dir: &Path, run_id: &str, machine: &str, trigger: &str) -> io::Result<Self> {
        let mut file = OpenOptions::new()
            .read(true)
            .append(true)
            .create(true)
            .open(run_dir.join(EVENTS_FILE))?;
        let mut existing = Vec::new();
        file.read_to_end(&mut existing)?;
        let lines = existing
            .split(|&b| b == b'\n')
            .filter(|line| !line.trim_ascii().is_empty())
            .count();
        Ok(EventLog {
            file,
            next_seq: lines as u64 + 1,
            run_id: run_id.to_string(),
            machine: machine.to_string(),
            trigger: trigger.to_string(),
        })
    }

    /// Append one event of type `kind`: the fields every event carries, plus those of
    /// `fields`, a JSON object. Returns its `seq`.
    pub fn append(&mut self, kind: &str, fields: Value) -> io::Result<u64> {
        let Value::Object(fields) = fields else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "event fields are not a JSON object",
            ));
        };
        let seq = self.next_seq;
        let mut event = Map::new();
        event.insert("v".into(), json!(EVENTS_VERSION));
        event.insert("seq".into(), json!(seq));
        event.insert("ts".into(), json!(timestamp(Utc::now())));
        event.insert("type".into(), json!(kind));
        event.insert("run_id".into(), json!(self.run_id));
        event.insert("machine".into(), json!(self.machine));
        event.insert("trigger".into(), json!(self.trigger));
        event.extend(fields);
        let mut line = serde_json::to_vec(&Value::Object(event)).map_err(io::Error::other)?;
        line.push(b'\n');
        self.file.write_all(&line)?;
        self.next_seq += 1;
        Ok(seq)
    }
}

/// Every event in `runs/<id>/events.jsonl`, in order. A missing file holds no events.
pub fn read_events(run_dir: &Path) -> io::Result<Vec<Event>> {
    let text = match fs::read_to_string(run_dir.join(EVENTS_FILE)) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| match serde_json::from_str(line) {
            Ok(Value::Object(event)) => Ok(event),
            Ok(_) => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "event is not a JSON object",
            )),
            Err(e) => Err(io::Error::new(io::ErrorKind::InvalidData, e)),
        })
        .collect()
}

/// String field `key` of `event`, if it is one.
pub fn text<'e>(event: &'e Event, key: &str) -> Option<&'e str> {
    event.get(key).and_then(Value::as_str)
}

/// Whether `event` is of type `kind`.
pub fn is_type(event: &Event, kind: &str) -> bool {
    text(event, "type") == Some(kind)
}

/// The strings in list field `key` of `event`, such as a wait's `options`.
pub fn strings<'e>(event: &'e Event, key: &str) -> Vec<&'e str> {
    event
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect()
}

pub fn is_transition(event: &Event) -> bool {
    is_type(event, "transition")
}

/// `visits.<state>` (docs/reference/runs.md, Visits): the `transition` events whose `to` is that state,
/// except `source: "attempt"`.
pub fn visits(events: &[Event]) -> BTreeMap<String, u32> {
    let mut visits = BTreeMap::new();
    for event in events.iter().filter(|e| is_transition(e)) {
        if text(event, "source") == Some("attempt") {
            continue;
        }
        if let Some(to) = text(event, "to") {
            *visits.entry(to.to_string()).or_insert(0) += 1;
        }
    }
    visits
}

/// Each state's confidence from its latest `decision` event, 0 when that event has none
/// (docs/reference/machines.md, Conditions). A state with no `decision` event is missing.
pub fn confidences(events: &[Event]) -> BTreeMap<String, f64> {
    let mut confidences = BTreeMap::new();
    for event in events.iter().filter(|e| is_type(e, "decision")) {
        if let Some(state) = text(event, "state") {
            let confidence = event
                .get("confidence")
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            confidences.insert(state.to_string(), confidence);
        }
    }
    confidences
}

/// The run's state: the `to` of the last `transition` event (docs/reference/messages.md, Source of truth).
pub fn current_state(events: &[Event]) -> Option<&str> {
    events
        .iter()
        .rev()
        .find(|e| is_transition(e))
        .and_then(|e| text(e, "to"))
}

/// The child run a run waits for: the `child` of its last event, if that is `waiting`.
pub fn waiting_child(events: &[Event]) -> Option<&str> {
    events
        .last()
        .filter(|e| is_type(e, "waiting"))
        .and_then(|e| text(e, "child"))
}

/// The claim event: the first `transition`, with `source: "claim"`.
pub fn claim_event(events: &[Event]) -> Option<&Event> {
    events
        .iter()
        .find(|e| is_transition(e) && text(e, "source") == Some("claim"))
}

/// String field `key` of a run's first event, which names its `machine` and `trigger`.
pub fn first_text<'e>(events: &'e [Event], key: &str) -> Option<&'e str> {
    events.first().and_then(|e| text(e, key))
}
