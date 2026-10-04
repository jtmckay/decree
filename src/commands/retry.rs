//! `decree retry <id> [--state <s>]` (docs/reference/cli.md): make an `interrupted` or finished
//! run `pending` again. Appends a `transition` event with `source: "retry"` and mirrors
//! `state`; the next `process` or `daemon` pass continues the run, re-running root
//! `onentry` and the `onentry` of every ancestor of `<s>` and of `<s>` (docs/reference/runs.md, step 1).

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use serde_json::json;

use crate::cli::Format;
use crate::commands::check::Project;
use crate::commands::print_json;
use crate::commands::process::context;
use crate::error::DecreeError;
use crate::events::{current_state, first_text, is_transition, text, Event, EventLog};
use crate::interpreter::recover::{mirror_state, RunStatus};
use crate::layout::MESSAGE_FILE;
use crate::message::{is_valid_id, RunLock};

/// Run `decree retry`.
pub fn run(
    project_root: &Path,
    id: &str,
    state: Option<&str>,
    format: Format,
) -> Result<(), DecreeError> {
    let project = Project::load(project_root)?;
    let ctx = context(project_root, &project, Arc::new(AtomicBool::new(false)));
    let run_dir = ctx.runs_dir().join(id);
    if !is_valid_id(id) || !run_dir.is_dir() {
        return Err(DecreeError::MessageNotFound(id.to_string()));
    }
    // Hold the lock while writing, so no process steps the run meanwhile.
    let Some(_lock) = RunLock::acquire(&run_dir)? else {
        return Err(refuse(id, RunStatus::Active));
    };
    let events = ctx.events(id)?;
    let machine_name = first_text(&events, "machine").unwrap_or_default();
    let Some(machine) = project.machines.get(machine_name) else {
        return Err(DecreeError::Other(format!(
            "run {id}: machine `{machine_name}` is not loaded; it cannot be continued"
        )));
    };
    // This process holds the lock, so no other is stepping the run.
    let status = ctx.status(machine, &events, false);
    match status {
        RunStatus::Interrupted | RunStatus::Finished => {}
        RunStatus::Waiting => {
            return Err(DecreeError::Other(format!(
                "run {id} is waiting for a reply, not interrupted; answer it with `decree event`"
            )))
        }
        RunStatus::Pending | RunStatus::Active => return Err(refuse(id, status)),
    }

    let from = current_state(&events);
    let target = match (state, status) {
        (Some(s), _) => s.to_string(),
        (None, RunStatus::Finished) => last_from(&events).ok_or_else(|| {
            DecreeError::Other(format!(
                "run {id} never left a state; name one with --state"
            ))
        })?,
        (None, _) => from
            .ok_or_else(|| {
                DecreeError::Other(format!("run {id} has no state; name one with --state"))
            })?
            .to_string(),
    };
    let atomic = machine.find(&target).is_some_and(|n| {
        let node = &machine.nodes[n];
        node.children.is_empty() && !node.is_final
    });
    if !atomic {
        return Err(DecreeError::Other(format!(
            "`{target}` is not an atomic state of machine `{machine_name}`"
        )));
    }

    let trigger = first_text(&events, "trigger").unwrap_or_default();
    let mut log = EventLog::open(&run_dir, id, machine_name, trigger)?;
    log.append(
        "transition",
        json!({
            "from": from,
            "event": "retry",
            "to": target,
            "source": "retry",
            "exit_code": null,
        }),
    )?;
    mirror_state(&run_dir.join(MESSAGE_FILE), &target)?;
    match format {
        Format::Text => {
            println!("run {id} is pending in `{target}`; `decree process` continues it");
            Ok(())
        }
        Format::Json => print_json(&json!({ "id": id, "state": target })),
    }
}

/// The `from` of the last `transition` event that has one.
fn last_from(events: &[Event]) -> Option<String> {
    events
        .iter()
        .rev()
        .filter(|e| is_transition(e))
        .find_map(|e| text(e, "from"))
        .map(String::from)
}

fn refuse(id: &str, status: RunStatus) -> DecreeError {
    DecreeError::Other(format!(
        "run {id} is {}; only an interrupted or finished run can be retried",
        status.as_str()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_last_from_skips_transitions_without_from() {
        let events: Vec<_> = [
            json!({"type": "transition", "from": null, "to": "a"}),
            json!({"type": "transition", "from": "a", "to": "b"}),
            json!({"type": "script", "state": "b"}),
            json!({"type": "transition", "from": "b", "to": "failed"}),
            json!({"type": "run_finished", "state": "failed"}),
        ]
        .into_iter()
        .map(|v| v.as_object().cloned().unwrap())
        .collect();
        assert_eq!(last_from(&events).as_deref(), Some("b"));
        assert_eq!(last_from(&events[..1]), None);
    }
}
