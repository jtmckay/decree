//! `events.jsonl` (docs/reference/runs.md): the run's record. `EventLog` appends to it; the
//! functions below read it back and derive what the reference derives from it: the run's
//! state, visits and decision confidences.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::FileExt;
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
        .map(|line| parse_event(line.as_bytes()))
        .collect()
}

/// How many bytes `last_event` reads at a time, from the end.
const BLOCK: u64 = 4096;

/// The last event in `runs/<id>/events.jsonl`, read backwards from the end of the file a
/// block at a time until a whole line is found, so a long log costs one small read. A
/// missing or empty file holds none.
pub fn last_event(run_dir: &Path) -> io::Result<Option<Event>> {
    let file = match File::open(run_dir.join(EVENTS_FILE)) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    // `tail` holds the bytes from `start` to the end of the file.
    let mut start = file.metadata()?.len();
    let mut tail: Vec<u8> = Vec::new();
    loop {
        // The last line with anything but whitespace in it, once its start is read.
        if let Some(end) = tail.iter().rposition(|b| !b.is_ascii_whitespace()) {
            match tail[..end].iter().rposition(|&b| b == b'\n') {
                Some(newline) => return parse_event(&tail[newline + 1..=end]).map(Some),
                None if start == 0 => return parse_event(&tail[..=end]).map(Some),
                None => {}
            }
        } else if start == 0 {
            return Ok(None);
        }
        let from = start.saturating_sub(BLOCK);
        let mut block = vec![0; (start - from) as usize];
        file.read_exact_at(&mut block, from)?;
        block.extend_from_slice(&tail);
        tail = block;
        start = from;
    }
}

/// One line of `events.jsonl`: a JSON object.
fn parse_event(line: &[u8]) -> io::Result<Event> {
    match serde_json::from_slice(line) {
        Ok(Value::Object(event)) => Ok(event),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "event is not a JSON object",
        )),
        Err(e) => Err(io::Error::new(io::ErrorKind::InvalidData, e)),
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// `last_event` of an `events.jsonl` holding `text`.
    fn last_of(text: &[u8]) -> io::Result<Option<Event>> {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join(EVENTS_FILE), text).unwrap();
        last_event(dir.path())
    }

    fn seq(event: Option<Event>) -> Option<u64> {
        event.and_then(|e| e.get("seq").and_then(Value::as_u64))
    }

    #[test]
    fn test_last_event_of_one_line() {
        assert_eq!(seq(last_of(b"{\"seq\":1}\n").unwrap()), Some(1));
    }

    #[test]
    fn test_last_event_of_many_lines_across_blocks() {
        let text: String = (1..=1000).map(|n| format!("{{\"seq\":{n}}}\n")).collect();
        assert!(text.len() as u64 > 2 * BLOCK);
        assert_eq!(seq(last_of(text.as_bytes()).unwrap()), Some(1000));
    }

    #[test]
    fn test_last_event_without_a_final_newline() {
        assert_eq!(seq(last_of(b"{\"seq\":1}\n{\"seq\":2}").unwrap()), Some(2));
        // Blank lines and `\r\n` endings at the end are not events.
        assert_eq!(
            seq(last_of(b"{\"seq\":1}\r\n{\"seq\":2}\r\n\n  \n").unwrap()),
            Some(2)
        );
    }

    #[test]
    fn test_last_event_longer_than_a_block() {
        let long = "x".repeat(3 * BLOCK as usize);
        let text = format!("{{\"seq\":1}}\n{{\"seq\":2,\"log\":\"{long}\"}}\n");
        let last = last_of(text.as_bytes()).unwrap().unwrap();
        assert_eq!(last["log"], json!(long));
        assert_eq!(seq(Some(last)), Some(2));
        // The only line, longer than a block.
        let only = format!("{{\"seq\":1,\"log\":\"{long}\"}}");
        assert_eq!(seq(last_of(only.as_bytes()).unwrap()), Some(1));
    }

    #[test]
    fn test_last_event_of_an_empty_or_missing_file() {
        assert_eq!(last_of(b"").unwrap(), None);
        assert_eq!(last_of(b"\n \n").unwrap(), None);
        let dir = TempDir::new().unwrap();
        assert_eq!(last_event(dir.path()).unwrap(), None);
    }

    #[test]
    fn test_last_event_reads_only_the_last_line() {
        assert_eq!(
            seq(last_of(b"{\"seq\":1}\nnot json\n{\"seq\":3}\n").unwrap()),
            Some(3)
        );
        let err = last_of(b"{\"seq\":1}\nnot json\n").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }
}
