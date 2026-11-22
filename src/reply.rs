//! Replies to runs waiting in a `reply` state (docs/reference/messages.md, Replies): the
//! checks `decree reply` makes before writing a reply, delivery when `process` claims one,
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
use crate::message::{self, lock_state, run_ids, LockState, Message, RunLock, LOCK_FILE};

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

/// Whether a reply addressed to run id `to` (not a wait id) may be queued for a run that is
/// not waiting now but will be: it is `active` or `pending`, and `event` is an option of some
/// `reply` state of its machine. Delivery holds it until the run waits (docs/reference/
/// messages.md, Replies); one the wait then does not take is rejected as any is.
pub fn check_later(ctx: &Context, to: &str, event: &str) -> Result<Result<(), String>, InterpreterError> {
    let runs_dir = ctx.runs_dir();
    let Some((run_id, None)) = resolve(&runs_dir, to) else {
        return Ok(Err(format!("`{to}` is not a run id")));
    };
    let (status, events) = ctx.status_of(run_id)?;
    if !matches!(status, RunStatus::Active | RunStatus::Pending) {
        return Ok(Err(format!("run {run_id} is {}", status.as_str())));
    }
    let machine = first_text(&events, "machine").unwrap_or_default();
    let Some(m) = ctx.machines.get(machine) else {
        return Ok(Err(format!("run {run_id}'s machine `{machine}` does not load")));
    };
    let takes = (0..m.nodes.len())
        .filter(|&i| matches!(m.nodes[i].invoke, Some(crate::machine::Invoke::Reply(_))))
        .any(|i| m.accepted_events(i).iter().any(|e| e == event));
    if takes {
        Ok(Ok(()))
    } else {
        Ok(Err(format!(
            "no `reply` state of machine `{machine}` takes `{event}`"
        )))
    }
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
    // The lock before the log: a delivery appends `received` before it releases the lock,
    // so a run seen unlocked here shows any delivery that held it.
    let alive = !locked
        && matches!(
            lock_state(&run_dir).map_err(io_err(&run_dir.join(LOCK_FILE)))?,
            LockState::Live(_)
        );
    let events = ctx.events(run_id)?;
    let machine = first_text(&events, "machine").unwrap_or_default();
    let Some(m) = ctx.machines.get(machine) else {
        return Ok(Err(format!(
            "run {run_id} is not waiting: its machine `{machine}` does not load"
        )));
    };
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

/// The replies queued in `inbox_dir`, in filename order, as (filename, `to`). Unreadable
/// files are skipped.
pub fn queued(inbox_dir: &Path) -> io::Result<Vec<(String, String)>> {
    let entries = match fs::read_dir(inbox_dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut names = Vec::new();
    for entry in entries {
        let Ok(name) = entry?.file_name().into_string() else {
            continue;
        };
        if name.ends_with(".md") && !name.starts_with('.') {
            names.push(name);
        }
    }
    names.sort();
    Ok(names
        .into_iter()
        .filter_map(|name| {
            let to = Message::read(&inbox_dir.join(&name))
                .ok()?
                .text("to")?
                .to_string();
            Some((name, to))
        })
        .collect())
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

/// The run a reply's `to` names, its wait id or run id; `None` when it names no run.
pub fn target_run(runs_dir: &Path, to: &str) -> Option<String> {
    resolve(runs_dir, to).map(|(run_id, _)| run_id.to_string())
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
        ctx,
        &wait,
        json!({ "wait_id": wait.wait_id, "event": event, "file": file }),
    )?;
    Ok(Delivery::Delivered(wait.run_id))
}

/// A run waiting for a reply past its `timeout_at`, found without its lock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Overdue {
    pub run_id: String,
    pub wait_id: String,
    pub timeout_at: DateTime<Utc>,
}

impl Overdue {
    /// Whether a reply with `to` and id `reply_id`, queued in `inbox/`, wins over this
    /// timeout (docs/reference/messages.md, Replies, step 5): it names this wait (its wait
    /// id, or the run id meaning its current wait) and was queued at or before the deadline.
    /// The time that counts is the reply's id, when it reached the queue, not when `process`
    /// gets to it.
    pub fn answered_by(&self, to: &str, reply_id: &str) -> bool {
        (to == self.run_id || to == self.wait_id)
            && message::id_time(reply_id).is_some_and(|t| t <= self.timeout_at)
    }
}

/// Every run, in `id` order, that `owns` is true for and that waits for a reply past its
/// `timeout_at` at `now`.
pub fn overdue(
    ctx: &Context,
    now: DateTime<Utc>,
    owns: impl Fn(&str) -> bool,
) -> Result<Vec<Overdue>, InterpreterError> {
    let runs_dir = ctx.runs_dir();
    let mut overdue = Vec::new();
    for id in run_ids(&runs_dir).map_err(io_err(&runs_dir))? {
        if ctx.run_finished(&id)?.is_some() || !owns(&id) {
            continue;
        }
        let events = ctx.events(&id)?;
        let Some(last) = events.last().filter(|&e| past_deadline(Some(e), now)) else {
            continue;
        };
        let (Some(wait_id), Some(timeout_at)) = (text(last, "wait_id"), deadline(last)) else {
            continue;
        };
        overdue.push(Overdue {
            run_id: id,
            wait_id: wait_id.to_string(),
            timeout_at,
        });
    }
    Ok(overdue)
}

/// The earliest `timeout_at` of a run that `owns` is true for and that waits only on a timer:
/// a reply wait with `attention: false` and a `timeout` (docs/reference/cli.md, `daemon
/// --once`). A wait for a person or a program is no timer: `--once` leaves it for the summary.
pub fn next_timer(
    ctx: &Context,
    owns: impl Fn(&str) -> bool,
) -> Result<Option<DateTime<Utc>>, InterpreterError> {
    let runs_dir = ctx.runs_dir();
    let mut next: Option<DateTime<Utc>> = None;
    for id in run_ids(&runs_dir).map_err(io_err(&runs_dir))? {
        if ctx.run_finished(&id)?.is_some() || !owns(&id) {
            continue;
        }
        let events = ctx.events(&id)?;
        let Some(last) = events.last() else { continue };
        let timer = is_type(last, "waiting")
            && text(last, "wait_id").is_some()
            && !crate::attention::wants_attention(last);
        if let Some(at) = deadline(last).filter(|_| timer) {
            next = Some(next.map_or(at, |n| n.min(at)));
        }
    }
    Ok(next)
}

/// docs/reference/messages.md, Replies, step 5: every run waiting past its `timeout_at` gets a `received`
/// event for `error` with `timed_out: true`. Returns those runs, in `id` order, now
/// `pending`. A run another process holds is skipped, as is one `owns` is false for: a run
/// of a machine this process does not run. The caller delivers first any reply queued
/// before a deadline (`Overdue::answered_by`), which wins over the timeout.
pub fn deliver_timeouts(
    ctx: &Context,
    now: DateTime<Utc>,
    owns: impl Fn(&str) -> bool,
) -> Result<Vec<String>, InterpreterError> {
    let runs_dir = ctx.runs_dir();
    let mut timed_out = Vec::new();
    for Overdue { run_id: id, .. } in overdue(ctx, now, owns)? {
        let run_dir = runs_dir.join(&id);
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
            ctx,
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
        && deadline(last).is_some_and(|deadline| deadline <= now)
}

/// The `timeout_at` of a `waiting` event.
fn deadline(waiting: &Event) -> Option<DateTime<Utc>> {
    text(waiting, "timeout_at")
        .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
        .map(|t| t.with_timezone(&Utc))
}

fn append_received(ctx: &Context, wait: &Wait, fields: Value) -> Result<(), InterpreterError> {
    let path = wait.run_dir.join(EVENTS_FILE);
    let mut log = EventLog::open(&wait.run_dir, &wait.run_id, &wait.machine, &wait.trigger)
        .map_err(io_err(&path))?;
    log.append("received", fields).map_err(io_err(&path))?;
    // The run is `pending`: it needs no one now.
    crate::attention::remove(&ctx.project_root, &wait.run_id)
}
