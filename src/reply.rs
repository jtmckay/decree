//! Replies to runs waiting in a `choose: person` state (docs/reference/messages.md, Replies): the
//! checks `decree event` makes before writing a reply, delivery when `process` claims one,
//! and `timeout_s` deadlines. Both deliveries append a `received` event under the run lock,
//! which makes the run `pending`; the caller then continues it.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde_json::{json, Map, Value};

use crate::interpreter::{read_events, run_status, RunStatus};
use crate::machine::LoadedMachine;
use crate::message::{self, lock_state, LockState, RunLock, LOCK_FILE};
use crate::runtime::{EventLog, EVENTS_FILE, RECEIVED_DIR};

#[derive(Debug, thiserror::Error)]
pub enum ReplyError {
    #[error("{}: {source}", path.display())]
    Io { path: PathBuf, source: io::Error },
}

fn io_err(path: &Path) -> impl FnOnce(io::Error) -> ReplyError + '_ {
    move |source| ReplyError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// The wait a reply answers: a run whose status is `waiting` for a reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wait {
    pub run_id: String,
    pub run_dir: PathBuf,
    /// `<run id>.w<seq>`, from the run's last event.
    pub wait_id: String,
    pub options: Vec<String>,
    machine: String,
    trigger: String,
}

/// Find the wait `to` names (a wait id, or a run id meaning its current wait) and check
/// that `event` is one of its options. `Ok(Err(reason))` is a reply that fails a check
/// (docs/reference/messages.md, Replies, step 4): unknown run, run not waiting, stale wait id, unknown
/// option. `locked` means the caller holds the run's lock, so its own lock is not a live one.
pub fn check(
    runs_dir: &Path,
    machines: &BTreeMap<String, LoadedMachine>,
    to: &str,
    event: &str,
    locked: bool,
) -> Result<Result<Wait, String>, ReplyError> {
    Ok(find_wait(runs_dir, machines, to, locked)?.and_then(|wait| {
        if wait.options.iter().any(|o| o == event) {
            Ok(wait)
        } else {
            Err(format!(
                "`{event}` is not an option of wait {}; options: {}",
                wait.wait_id,
                wait.options.join(", ")
            ))
        }
    }))
}

/// The wait `to` names, if its run is `waiting` for a reply and `to` is not stale.
fn find_wait(
    runs_dir: &Path,
    machines: &BTreeMap<String, LoadedMachine>,
    to: &str,
    locked: bool,
) -> Result<Result<Wait, String>, ReplyError> {
    let Some((run_id, wanted)) = resolve(runs_dir, to) else {
        return Ok(Err(format!("`to` {to} names no run")));
    };
    let run_dir = runs_dir.join(run_id);
    let events = read_events(&run_dir).map_err(io_err(&run_dir.join(EVENTS_FILE)))?;
    let first = |key: &str| {
        events
            .first()
            .and_then(|e| e.get(key))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let machine = first("machine");
    let Some(m) = machines.get(&machine) else {
        return Ok(Err(format!(
            "run {run_id} is not waiting: its machine `{machine}` does not load"
        )));
    };
    let alive = !locked
        && matches!(
            lock_state(&run_dir).map_err(io_err(&run_dir.join(LOCK_FILE)))?,
            LockState::Live(_)
        );
    let status = run_status(m, &events, alive);
    if status != RunStatus::Waiting {
        return Ok(Err(format!(
            "run {run_id} is not waiting: it is {}",
            status.as_str()
        )));
    }
    let last = events.last().expect("a waiting run has events");
    let Some(wait_id) = last.get("wait_id").and_then(Value::as_str) else {
        let child = last.get("child").and_then(Value::as_str).unwrap_or("?");
        return Ok(Err(format!(
            "run {run_id} waits for child run {child}, not for a reply"
        )));
    };
    if wanted.is_some_and(|w| w != wait_id) {
        return Ok(Err(format!(
            "stale wait id {to}: run {run_id} now waits as {wait_id}"
        )));
    }
    let options: Vec<String> = last
        .get("options")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(String::from)
        .collect();
    Ok(Ok(Wait {
        run_id: run_id.to_string(),
        run_dir,
        wait_id: wait_id.to_string(),
        options,
        machine,
        trigger: first("trigger"),
    }))
}

/// The run `to` names, and the wait id if `to` is one. A run id that is also a valid
/// `<run id>.w<seq>` is read as the run id, since that run exists.
fn resolve<'t>(runs_dir: &Path, to: &'t str) -> Option<(&'t str, Option<&'t str>)> {
    let is_run = |id: &str| message::is_valid_id(id) && runs_dir.join(id).is_dir();
    if is_run(to) {
        return Some((to, None));
    }
    let (run_id, seq) = to.rsplit_once(".w")?;
    let numeric = !seq.is_empty() && seq.bytes().all(|b| b.is_ascii_digit());
    (numeric && is_run(run_id)).then_some((run_id, Some(to)))
}

/// What delivering a reply did.
#[derive(Debug, PartialEq, Eq)]
pub enum Delivery {
    /// The reply was moved to `runs/<run id>/received/` and a `received` event appended:
    /// the run is `pending`.
    Delivered(String),
    /// The reply fails a check; it becomes a failed `invalid_message` run of its own.
    Rejected(String),
    /// The inbox file is gone: another process took it.
    Lost,
}

/// Deliver `inbox_dir/<file>`, a reply with frontmatter `to` and `event` (docs/reference/messages.md,
/// Replies, step 3). Under the run's lock: check it, move it to
/// `runs/<run id>/received/<file>`, and append a `received` event. The move never replaces
/// an earlier reply of the same filename, whose `decision` event names it.
pub fn deliver(
    runs_dir: &Path,
    machines: &BTreeMap<String, LoadedMachine>,
    inbox_dir: &Path,
    file: &str,
    to: Option<&str>,
    event: Option<&str>,
) -> Result<Delivery, ReplyError> {
    let (Some(to), Some(event)) = (to, event) else {
        return Ok(Delivery::Rejected(
            "a reply needs string `to` and `event` keys".to_string(),
        ));
    };
    let Some((run_id, _)) = resolve(runs_dir, to) else {
        return Ok(Delivery::Rejected(format!("`to` {to} names no run")));
    };
    let run_dir = runs_dir.join(run_id);
    let Some(_lock) = RunLock::acquire(&run_dir).map_err(io_err(&run_dir.join(LOCK_FILE)))? else {
        return Ok(Delivery::Rejected(format!(
            "run {run_id} is not waiting: it is active"
        )));
    };
    let wait = match check(runs_dir, machines, to, event, true)? {
        Ok(wait) => wait,
        Err(reason) => return Ok(Delivery::Rejected(reason)),
    };

    let received = run_dir.join(RECEIVED_DIR);
    fs::create_dir_all(&received).map_err(io_err(&received))?;
    let source = inbox_dir.join(file);
    let target = received.join(file);
    // A hard link fails rather than replace an existing file; then drop the inbox name.
    match fs::hard_link(&source, &target) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Delivery::Lost),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            return Ok(Delivery::Rejected(format!(
                "run {} already received a reply named {file}",
                wait.run_id
            )))
        }
        Err(e) => return Err(io_err(&target)(e)),
    }
    fs::remove_file(&source).map_err(io_err(&source))?;
    append_received(
        &wait,
        json!({ "wait_id": wait.wait_id, "event": event, "file": file }),
    )?;
    Ok(Delivery::Delivered(wait.run_id))
}

/// docs/reference/messages.md, Replies, step 5: every run waiting past its `timeout_at` gets a `received`
/// event for `error` with `timed_out: true`. Returns those runs, in `id` order, now
/// `pending`. A run another process holds is skipped.
pub fn deliver_timeouts(
    runs_dir: &Path,
    machines: &BTreeMap<String, LoadedMachine>,
    now: DateTime<Utc>,
) -> Result<Vec<String>, ReplyError> {
    let mut ids: Vec<String> = match fs::read_dir(runs_dir) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .filter(|e| e.path().is_dir())
            .filter_map(|e| e.file_name().into_string().ok())
            .collect(),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(io_err(runs_dir)(e)),
    };
    ids.sort();
    let mut timed_out = Vec::new();
    for id in ids {
        let run_dir = runs_dir.join(&id);
        let events = read_events(&run_dir).map_err(io_err(&run_dir.join(EVENTS_FILE)))?;
        if !past_deadline(events.last(), now) {
            continue;
        }
        let Some(_lock) = RunLock::acquire(&run_dir).map_err(io_err(&run_dir.join(LOCK_FILE)))?
        else {
            continue;
        };
        // Checked again under the lock: a reply may have arrived meanwhile.
        let events = read_events(&run_dir).map_err(io_err(&run_dir.join(EVENTS_FILE)))?;
        let Some(last) = events.last().filter(|&e| past_deadline(Some(e), now)) else {
            continue;
        };
        let wait_id = last
            .get("wait_id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let Ok(wait) = find_wait(runs_dir, machines, wait_id, true)? else {
            continue;
        };
        append_received(
            &wait,
            json!({ "wait_id": wait_id, "event": "error", "timed_out": true }),
        )?;
        timed_out.push(id);
    }
    Ok(timed_out)
}

/// Whether `last` is a `waiting` event for a reply whose `timeout_at` is at or before `now`.
fn past_deadline(last: Option<&Map<String, Value>>, now: DateTime<Utc>) -> bool {
    let Some(last) = last else { return false };
    last.get("type").and_then(Value::as_str) == Some("waiting")
        && last.get("wait_id").and_then(Value::as_str).is_some()
        && last
            .get("timeout_at")
            .and_then(Value::as_str)
            .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
            .is_some_and(|deadline| deadline <= now)
}

fn append_received(wait: &Wait, fields: Value) -> Result<(), ReplyError> {
    let path = wait.run_dir.join(EVENTS_FILE);
    let mut log = EventLog::open(&wait.run_dir, &wait.run_id, &wait.machine, &wait.trigger)
        .map_err(io_err(&path))?;
    let Value::Object(fields) = fields else {
        unreachable!("event fields are a JSON object");
    };
    log.append("received", fields).map_err(io_err(&path))?;
    Ok(())
}
