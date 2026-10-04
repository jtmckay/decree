//! Run status and recovery (docs/reference/messages.md, Lifecycle, Run status, Source of
//! truth): a run's status derived from its events, what `process` and `daemon` do with runs
//! a crash left behind, the `state` mirror in `message.md`, and the run of an invalid
//! message.

use std::path::Path;

use serde_json::json;

use super::{io_err, Context, InterpreterError};
use crate::events::{
    current_state, first_text, is_type, read_events, text, waiting_child, Event, EventLog,
    EVENTS_FILE,
};
use crate::layout::MESSAGE_FILE;
use crate::machine::{LoadedMachine, FAILED};
use crate::message::{lock_state, run_ids, LockState, Message, MessageError, LOCK_FILE};
use crate::runtime::{Running, RUNNING_FILE};

/// What `recover` found (docs/reference/messages.md, Run status).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Recovery {
    /// Runs that just got an `interrupted` event with `cause: "crash"`, with the state each
    /// was in, in `id` order.
    pub crashed: Vec<(String, String)>,
    /// `pending` runs, in `id` order: `process` and `daemon` continue them.
    pub pending: Vec<String>,
}

/// What `process` and `daemon` do when they start (docs/reference/messages.md, Run status). Every run that
/// is `interrupted` but whose last event is not already `interrupted` gets one with
/// `cause: "crash"`, so the stop is visible in the log; it is never continued. `active`
/// runs (a live pid in `.lock`) are left alone. Every other run's `message.md` mirror is
/// rewritten if it disagrees with `events.jsonl` (docs/reference/messages.md, Source of truth). Runs with
/// no events yet (never claimed past the folder), or of a machine that no longer exists,
/// are skipped: there is no state to record or continue.
pub fn recover(ctx: &Context) -> Result<Recovery, InterpreterError> {
    let runs = ctx.runs_dir();
    let mut found = Recovery::default();
    for id in run_ids(&runs).map_err(io_err(&runs))? {
        let run_dir = runs.join(&id);
        let events = ctx.events(&id)?;
        let Some(machine) = ctx.run_machine(&events) else {
            continue;
        };
        let lock = lock_state(&run_dir).map_err(io_err(&run_dir.join(LOCK_FILE)))?;
        if matches!(lock, LockState::Live(_)) {
            continue;
        }
        repair_mirror(&run_dir, &events)?;
        let last_interrupted = events.last().is_some_and(|e| is_type(e, "interrupted"));
        match ctx.status(machine, &events, false) {
            RunStatus::Interrupted if !last_interrupted => {
                let state = current_state(&events).unwrap_or_default().to_string();
                let path = run_dir.join(EVENTS_FILE);
                let trigger = first_text(&events, "trigger").unwrap_or_default();
                let mut log =
                    EventLog::open(&run_dir, &id, &machine.id, trigger).map_err(io_err(&path))?;
                let mut fields = json!({ "state": state, "cause": "crash" });
                // A `.running` the crash left behind names the script (docs/reference/scripts.md).
                let running_path = run_dir.join(RUNNING_FILE);
                if let Some(running) = Running::read(&run_dir).map_err(io_err(&running_path))? {
                    fields["script"] = json!(running.script);
                }
                log.append("interrupted", fields).map_err(io_err(&path))?;
                Running::remove(&run_dir).map_err(io_err(&running_path))?;
                found.crashed.push((id, state));
            }
            RunStatus::Pending => found.pending.push(id),
            _ => {}
        }
    }
    Ok(found)
}

/// Rewrite the `state` mirror in `run_dir`'s `message.md` if it disagrees with the run's
/// state, the `to` of the last `transition` event (docs/reference/messages.md, Source of truth). A message
/// whose frontmatter does not parse is left unchanged, as Lifecycle step 3 leaves it.
/// Returns whether it was rewritten.
pub fn repair_mirror(run_dir: &Path, events: &[Event]) -> Result<bool, InterpreterError> {
    let Some(state) = current_state(events) else {
        return Ok(false);
    };
    let path = run_dir.join(MESSAGE_FILE);
    let mut message = match Message::read(&path) {
        Ok(message) => message,
        Err(MessageError::Parse { .. }) => return Ok(false),
        Err(e) => return Err(e.into()),
    };
    if message.text("state") == Some(state) {
        return Ok(false);
    }
    message.set("state", state);
    message.write(&path)?;
    Ok(true)
}

/// docs/reference/messages.md, Run status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    Finished,
    Active,
    Waiting,
    Pending,
    Interrupted,
}

impl RunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            RunStatus::Finished => "finished",
            RunStatus::Active => "active",
            RunStatus::Waiting => "waiting",
            RunStatus::Pending => "pending",
            RunStatus::Interrupted => "interrupted",
        }
    }
}

/// A run's status, derived from its events in the docs/reference/messages.md order. `lock_alive` is whether
/// the run's `.lock` holds a live pid.
/// Only a root-level final state finishes a run: a nested one is passed through at once.
pub fn run_status(m: &LoadedMachine, events: &[Event], lock_alive: bool) -> RunStatus {
    let in_final = current_state(events)
        .and_then(|s| m.find(s))
        .is_some_and(|s| m.is_root_final(s));
    let last = events.last();
    let last_type = last.and_then(|e| text(e, "type"));
    let last_source = last.and_then(|e| text(e, "source"));
    if in_final {
        RunStatus::Finished
    } else if lock_alive {
        RunStatus::Active
    } else if last_type == Some("waiting") {
        RunStatus::Waiting
    } else if last_type == Some("received")
        || (last_type == Some("transition") && last_source == Some("retry"))
    {
        RunStatus::Pending
    } else {
        RunStatus::Interrupted
    }
}

/// docs/reference/messages.md, Lifecycle step 3: an invalid message starts its run in `failed`. Appends the
/// one `transition` event, and mirrors `state: failed` when `mirror` is set (it is not
/// when the frontmatter itself did not parse). Nothing runs.
pub fn reject(
    events: &mut EventLog,
    run_dir: &Path,
    file: &str,
    reason: &str,
    mirror: bool,
) -> Result<(), InterpreterError> {
    let fields = json!({
        "from": null,
        "event": "claimed",
        "to": FAILED,
        "source": "invalid_message",
        "exit_code": null,
        "file": file,
        "error": reason,
    });
    let path = run_dir.join(EVENTS_FILE);
    events.append("transition", fields).map_err(io_err(&path))?;
    if mirror {
        mirror_state(&run_dir.join(MESSAGE_FILE), FAILED)?;
    }
    Ok(())
}

/// Set frontmatter `state` in the message at `path`, keeping every other key, the key
/// order and the body bytes (docs/reference/messages.md, Parsing and writing).
pub fn mirror_state(path: &Path, state: &str) -> Result<(), InterpreterError> {
    let mut message = Message::read(path)?;
    message.set("state", state);
    Ok(message.write(path)?)
}

impl<'a> Context<'a> {
    /// A run's status (docs/reference/messages.md, Run status), where a run left `waiting` for a child that
    /// has already finished is `pending` (docs/reference/runs.md, Sub-machines).
    pub fn status(&self, m: &LoadedMachine, events: &[Event], lock_alive: bool) -> RunStatus {
        let status = run_status(m, events, lock_alive);
        let Some(child) = waiting_child(events).filter(|_| status == RunStatus::Waiting) else {
            return status;
        };
        match self.final_state(child) {
            Ok(Some(_)) => RunStatus::Pending,
            _ => status,
        }
    }

    /// Run `run_id`'s status (docs/reference/messages.md, Run status) and its events. A run
    /// whose machine is not loaded can never be stepped: it is `finished` in `failed` (an
    /// invalid message), else `active` while its lock is live, else `interrupted`.
    pub fn status_of(&self, run_id: &str) -> Result<(RunStatus, Vec<Event>), InterpreterError> {
        let run_dir = self.runs_dir().join(run_id);
        let events = self.events(run_id)?;
        let lock = lock_state(&run_dir).map_err(io_err(&run_dir.join(LOCK_FILE)))?;
        let alive = matches!(lock, LockState::Live(_));
        let status = match self.run_machine(&events) {
            Some(m) => self.status(m, &events, alive),
            None if current_state(&events) == Some(FAILED) => RunStatus::Finished,
            None if alive => RunStatus::Active,
            None => RunStatus::Interrupted,
        };
        Ok((status, events))
    }

    /// Every event of run `run_id`.
    pub fn events(&self, run_id: &str) -> Result<Vec<Event>, InterpreterError> {
        let dir = self.runs_dir().join(run_id);
        read_events(&dir).map_err(io_err(&dir.join(EVENTS_FILE)))
    }

    /// The machine a run's first event names, if it is loaded.
    pub fn run_machine(&self, events: &[Event]) -> Option<&'a LoadedMachine> {
        first_text(events, "machine").and_then(|name| self.machines.get(name))
    }

    /// The root final state run `run_id` reached, or `None` if it has not finished or its
    /// machine is not loaded.
    pub fn final_state(&self, run_id: &str) -> Result<Option<String>, InterpreterError> {
        let events = self.events(run_id)?;
        let finished = self
            .run_machine(&events)
            .is_some_and(|m| run_status(m, &events, false) == RunStatus::Finished);
        Ok(current_state(&events)
            .filter(|_| finished)
            .map(String::from))
    }
}
