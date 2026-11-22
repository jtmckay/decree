//! `decree cancel <id> [-m <note>] [--format <text|json>]` (docs/reference/cli.md, Cancel):
//! end a run that is `waiting`, `pending` or `interrupted`, and every unfinished child run
//! below it, as GitHub Actions' and Temporal's cancel do. Each ends in `failed`: a
//! `transition` with `event: "cancel"` and `source: "cancel"`, then `run_finished` with
//! `cancelled: true`. No script runs, as on a signal: a cancelled run's work stops where it
//! is. It gets no file in `.decree/attention/`, its own replies still queued are deleted, and
//! a parent waiting for it continues with `error`, as for any child run that failed. `decree
//! retry` continues a cancelled run, as any finished one.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde_json::json;

use crate::attention;
use crate::cli::Format;
use crate::commands::check::Project;
use crate::commands::print_json;
use crate::commands::process::context;
use crate::error::DecreeError;
use crate::events::{current_state, first_text, text, EventLog};
use crate::interpreter::recover::{mirror_state, RunStatus};
use crate::interpreter::{io_err, Context};
use crate::layout::{HOLD_FILE, INBOX_DIR, MESSAGE_FILE};
use crate::machine::FAILED;
use crate::message::{self, is_valid_id, Message, RunLock};
use crate::reply;

/// Run `decree cancel`: lock run `id` and each unfinished child run below it, check that none
/// is executing and `id` has not finished, then end each in `failed`, deepest first.
pub fn run(
    project_root: &Path,
    id: &str,
    note: Option<&str>,
    format: Format,
) -> Result<(), DecreeError> {
    let project = Project::load(project_root)?;
    let ctx = context(project_root, &project, Arc::new(AtomicBool::new(false)));
    let runs_dir = ctx.runs_dir();
    if !is_valid_id(id) || !runs_dir.join(id).is_dir() {
        return Err(DecreeError::MessageNotFound(id.to_string()));
    }
    if let Some(last) = ctx.run_finished(id)? {
        return Err(DecreeError::Other(format!(
            "run {id} has already finished in `{}`",
            text(&last, "state").unwrap_or_default()
        )));
    }
    let tree = unfinished_tree(&ctx, id)?;
    // Every lock first, so no process steps one of these runs, or continues a parent whose
    // child has just finished, while they are cancelled.
    let mut locks = Vec::new();
    for run in &tree {
        match RunLock::acquire(&runs_dir.join(run))? {
            Some(lock) => locks.push(lock),
            None => {
                return Err(DecreeError::Other(format!(
                    "run {run} is active: a script of it runs now; let it reach a wait, or \
                     interrupt the daemon (Ctrl-C), then cancel it"
                )))
            }
        }
    }
    // Under the locks, nothing is executing: check `id` again, as another process may have
    // finished it meanwhile.
    let (status, _) = ctx.status_of(id)?;
    if status == RunStatus::Finished {
        return Err(DecreeError::Other(format!("run {id} has already finished")));
    }
    let inbox = project.decree_dir.join(INBOX_DIR);
    let queued = reply::queued(&inbox).map_err(io_err(&inbox))?;
    let mut cancelled = Vec::new();
    for run in tree.iter().rev() {
        let state = cancel_one(&ctx, run, note)?;
        // A reply still queued for it would only be rejected now.
        for (file, to) in &queued {
            if reply::target_run(&runs_dir, to).as_deref() == Some(run.as_str()) {
                let path = inbox.join(file);
                match fs::remove_file(&path) {
                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                        return Err(io_err(&path)(e).into())
                    }
                    _ => {}
                }
            }
        }
        cancelled.push((run.clone(), state));
    }
    drop(locks);
    if format == Format::Json {
        let ids: Vec<&String> = cancelled.iter().map(|(run, _)| run).collect();
        return print_json(&json!({ "cancelled": ids }));
    }
    for (run, state) in &cancelled {
        println!("cancelled run {run} in `{state}`");
    }
    let parent = Message::read(&runs_dir.join(id).join(MESSAGE_FILE))
        .ok()
        .and_then(|m| m.text("parent").map(String::from));
    if let Some(parent) = parent {
        println!("its parent run {parent} continues with `error`, as for a child run that failed");
    }
    Ok(())
}

/// Run `id` and every unfinished run below it (a child run names its parent in `parent:`),
/// parents before their children.
fn unfinished_tree(ctx: &Context, id: &str) -> Result<Vec<String>, DecreeError> {
    let runs_dir = ctx.runs_dir();
    let mut children: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for run in message::run_ids(&runs_dir)? {
        let parent = Message::read(&runs_dir.join(&run).join(MESSAGE_FILE))
            .ok()
            .and_then(|m| m.text("parent").map(String::from));
        if let Some(parent) = parent {
            children.entry(parent).or_default().push(run);
        }
    }
    let mut tree = vec![id.to_string()];
    let mut i = 0;
    while i < tree.len() {
        for child in children.get(&tree[i]).into_iter().flatten() {
            if ctx.run_finished(child)?.is_none() {
                tree.push(child.clone());
            }
        }
        i += 1;
    }
    Ok(tree)
}

/// End run `id` in `failed`, its lock held by the caller: the `cancel` transition, the mirror,
/// `run_finished` with `cancelled: true`; then its `hold` and attention file go. Returns the
/// state it was in.
fn cancel_one(ctx: &Context, id: &str, note: Option<&str>) -> Result<String, DecreeError> {
    let run_dir = ctx.runs_dir().join(id);
    let events = ctx.events(id)?;
    let machine = first_text(&events, "machine").unwrap_or_default();
    let trigger = first_text(&events, "trigger").unwrap_or_default();
    let from = current_state(&events).map(String::from);
    let mut log = EventLog::open(&run_dir, id, machine, trigger)?;
    let mut fields = json!({
        "from": from,
        "event": "cancel",
        "to": FAILED,
        "source": "cancel",
        "exit_code": null,
    });
    if let Some(note) = note {
        fields["note"] = json!(note);
    }
    log.append("transition", fields)?;
    if run_dir.join(MESSAGE_FILE).is_file() {
        mirror_state(&run_dir.join(MESSAGE_FILE), FAILED)?;
    }
    let claimed = events
        .first()
        .and_then(|e| text(e, "ts"))
        .and_then(|ts| DateTime::parse_from_rfc3339(ts).ok());
    let duration_ms = claimed.map_or(0, |at| {
        (Utc::now() - at.with_timezone(&Utc))
            .num_milliseconds()
            .max(0) as u64
    });
    log.append(
        "run_finished",
        json!({ "state": FAILED, "duration_ms": duration_ms, "cancelled": true }),
    )?;
    let hold = run_dir.join(HOLD_FILE);
    match fs::remove_file(&hold) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(io_err(&hold)(e).into()),
        _ => {}
    }
    attention::remove(&ctx.project_root, id)?;
    Ok(from.unwrap_or_else(|| "-".to_string()))
}
