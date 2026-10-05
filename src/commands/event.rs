//! `decree event <wait id | run id> <event> [-m <note>]` (docs/reference/cli.md): queue a reply
//! for a run waiting in a `person` state (docs/reference/messages.md, Replies), through the same
//! writer as `decree emit`. The next `process` pass delivers it.

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::cli::Format;
use crate::commands::check::Project;
use crate::commands::emit::print_queued;
use crate::commands::process::context;
use crate::error::DecreeError;
use crate::layout::{DECREE_DIR, INBOX_DIR};
use crate::message::{self, Message};
use crate::reply;

/// Run `decree event`: check that the run is waiting, accepts `event` and has no reply
/// queued already, so mistakes fail at once, then queue the reply with the note as its
/// body. Prints the reply's id.
pub fn run(
    project_root: &Path,
    target: &str,
    event: &str,
    note: Option<&str>,
    format: Format,
) -> Result<(), DecreeError> {
    let project = Project::load(project_root)?;
    let ctx = context(project_root, &project, Arc::new(AtomicBool::new(false)));
    let wait = reply::check(&ctx, target, event, false)?.map_err(DecreeError::Other)?;
    let inbox = project.decree_dir.join(INBOX_DIR);
    if let Some(file) = reply::queued(&inbox, &wait)? {
        return Err(DecreeError::Other(format!(
            "wait {} already has a reply queued: {DECREE_DIR}/{INBOX_DIR}/{file}",
            wait.wait_id
        )));
    }

    let body = match note {
        Some(note) if !note.ends_with('\n') => format!("{note}\n"),
        Some(note) => note.to_string(),
        None => String::new(),
    };
    let mut message = Message::new(body);
    message.set("to", target);
    message.set("event", event);
    let id = message::queue(&project.decree_dir, &mut message)?;
    print_queued(&id, format)
}
