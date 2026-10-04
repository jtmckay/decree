//! Replies to runs waiting in a `person` state (docs/reference/messages.md, Replies): the
//! checks `decree event` makes before writing a reply, delivery when `process` claims one,
//! and `timeout` deadlines. Both deliveries append a `received` event under the run lock,
//! which makes the run `pending`; the caller then continues it.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use crate::events::{first_text, is_type, strings, text, Event, EventLog, EVENTS_FILE};
use crate::interpreter::recover::{run_status, RunStatus};
use crate::interpreter::{io_err, Context, InterpreterError};
use crate::layout::RECEIVED_DIR;
use crate::message::{self, lock_state, run_ids, LockState, RunLock, LOCK_FILE};

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
    ctx: &Context,
    to: &str,
    event: &str,
    locked: bool,
) -> Result<Result<Wait, String>, InterpreterError> {
    Ok(find_wait(ctx, to, locked)?.and_then(|wait| {
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
    ctx: &Context,
    to: &str,
    locked: bool,
) -> Result<Result<Wait, String>, InterpreterError> {
    let runs_dir = ctx.runs_dir();
    let Some((run_id, wanted)) = resolve(&runs_dir, to) else {
        return Ok(Err(format!("`to` {to} names no run")));
    };
    let run_dir = runs_dir.join(run_id);
    let events = ctx.events(run_id)?;
    let machine = first_text(&events, "machine").unwrap_or_default();
    let Some(m) = ctx.machines.get(machine) else {
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
    // `waiting` is the status of a run whose last event is `waiting`.
    let Some(last) = events.last() else {
        return Ok(Err(format!("run {run_id} has no events")));
    };
    let Some(wait_id) = text(last, "wait_id") else {
        let child = text(last, "child").unwrap_or("?");
        return Ok(Err(format!(
            "run {run_id} waits for child run {child}, not for a reply"
        )));
    };
    if wanted.is_some_and(|w| w != wait_id) {
        return Ok(Err(format!(
            "stale wait id {to}: run {run_id} now waits as {wait_id}"
        )));
    }
    Ok(Ok(Wait {
        run_id: run_id.to_string(),
        run_dir,
        wait_id: wait_id.to_string(),
        options: strings(last, "options")
            .into_iter()
            .map(String::from)
            .collect(),
        machine: machine.to_string(),
        trigger: first_text(&events, "trigger")
            .unwrap_or_default()
            .to_string(),
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
    ctx: &Context,
    inbox_dir: &Path,
    file: &str,
    to: Option<&str>,
    event: Option<&str>,
) -> Result<Delivery, InterpreterError> {
    let (Some(to), Some(event)) = (to, event) else {
        return Ok(Delivery::Rejected(
            "a reply needs string `to` and `event` keys".to_string(),
        ));
    };
    let runs_dir = ctx.runs_dir();
    let Some((run_id, _)) = resolve(&runs_dir, to) else {
        return Ok(Delivery::Rejected(format!("`to` {to} names no run")));
    };
    let run_dir = runs_dir.join(run_id);
    let Some(_lock) = RunLock::acquire(&run_dir).map_err(io_err(&run_dir.join(LOCK_FILE)))? else {
        return Ok(Delivery::Rejected(format!(
            "run {run_id} is not waiting: it is active"
        )));
    };
    let wait = match check(ctx, to, event, true)? {
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
    ctx: &Context,
    now: DateTime<Utc>,
) -> Result<Vec<String>, InterpreterError> {
    let runs_dir = ctx.runs_dir();
    let mut timed_out = Vec::new();
    for id in run_ids(&runs_dir).map_err(io_err(&runs_dir))? {
        let run_dir = runs_dir.join(&id);
        if ctx.run_finished(&id)?.is_some() || !past_deadline(ctx.events(&id)?.last(), now) {
            continue;
        }
        let Some(_lock) = RunLock::acquire(&run_dir).map_err(io_err(&run_dir.join(LOCK_FILE)))?
        else {
            continue;
        };
        // Checked again under the lock: a reply may have arrived meanwhile.
        let events = ctx.events(&id)?;
        let Some(last) = events.last().filter(|&e| past_deadline(Some(e), now)) else {
            continue;
        };
        let wait_id = text(last, "wait_id").unwrap_or_default();
        let Ok(wait) = find_wait(ctx, wait_id, true)? else {
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
fn past_deadline(last: Option<&Event>, now: DateTime<Utc>) -> bool {
    let Some(last) = last else { return false };
    is_type(last, "waiting")
        && text(last, "wait_id").is_some()
        && text(last, "timeout_at")
            .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
            .is_some_and(|deadline| deadline <= now)
}

fn append_received(wait: &Wait, fields: Value) -> Result<(), InterpreterError> {
    let path = wait.run_dir.join(EVENTS_FILE);
    let mut log = EventLog::open(&wait.run_dir, &wait.run_id, &wait.machine, &wait.trigger)
        .map_err(io_err(&path))?;
    log.append("received", fields).map_err(io_err(&path))?;
    Ok(())
}
