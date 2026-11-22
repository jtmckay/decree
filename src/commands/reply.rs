//! `decree reply <wait id | run id> <event> [-m <note>]` (docs/reference/cli.md): queue a reply
//! for a run waiting in a `reply` state, or about to (docs/reference/messages.md, Replies),
//! through the same writer as `decree emit`. The next `daemon` pass that finds the run
//! waiting delivers it.

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

/// Run `decree reply`: check that the run is waiting and accepts `event`, so mistakes fail
/// at once, then queue the reply with the note as its body. Prints the reply's id. A reply
/// to a run id may also be queued while the run is `active` or `pending` (it will wait
/// again), when a `reply` state of its machine takes `event`: it is delivered once the run
/// waits, as replies to one run are, in the order they were queued. Only a second reply to
/// the same wait id is refused, since one wait takes one reply.
pub fn run(
    project_root: &Path,
    target: &str,
    event: &str,
    note: Option<&str>,
    format: Format,
) -> Result<(), DecreeError> {
    let project = Project::load(project_root)?;
    let ctx = context(project_root, &project, Arc::new(AtomicBool::new(false)));
    // The inbox before the run: a reply gone from `inbox/` by now was delivered under the
    // run's lock, so the check below sees the run `active` or no longer waiting.
    let queued = reply::queued(&project.decree_dir.join(INBOX_DIR))?;
    match reply::check(&ctx, target, event, false)? {
        Ok(wait) => {
            // A wait id names one wait: a second reply to it could never be delivered.
            if target == wait.wait_id {
                if let Some((file, _)) = queued.iter().find(|(_, to)| *to == wait.wait_id) {
                    return Err(DecreeError::Other(format!(
                        "wait {} already has a reply queued: {DECREE_DIR}/{INBOX_DIR}/{file}",
                        wait.wait_id
                    )));
                }
            }
        }
        Err(not_waiting) => {
            if let Err(why) = reply::check_later(&ctx, target, event)? {
                return Err(DecreeError::Other(format!("{not_waiting} ({why})")));
            }
        }
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
