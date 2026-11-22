//! `decree pause [-m <note>]` and `decree resume` (docs/reference/cli.md, Pausing): pause
//! writes `.decree/attention/pause.md`, which holds the queue (docs/reference/cli.md,
//! Attention): the runs going now finish their current step loop and no run starts or
//! continues. A file already there is left as it is. resume deletes it, and a daemon goes on
//! without a restart. decree writes nothing else for them: no pid list, no signal, so they
//! reach every daemon of the project.

use std::path::Path;

use serde_json::json;

use crate::attention::{self, Paused, PAUSE_FILE};
use crate::cli::Format;
use crate::commands::print_json;
use crate::error::DecreeError;
use crate::layout::{ATTENTION_DIR, DECREE_DIR};

/// `.decree/attention/pause.md`, from the project root.
fn path() -> String {
    format!("{DECREE_DIR}/{ATTENTION_DIR}/{PAUSE_FILE}")
}

/// Run `decree pause`: write the file, or say it is there.
pub fn pause(project_root: &Path, note: Option<&str>, format: Format) -> Result<(), DecreeError> {
    let path = path();
    let paused = attention::pause(project_root, note, chrono::Utc::now())
        .map_err(|e| DecreeError::Other(format!("cannot write {path}: {e}")))?;
    if format == Format::Json {
        return print_json(&json!({ "attention": path, "paused": true }));
    }
    match paused {
        Paused::Written => println!(
            "paused: the runs executing now finish their step, and nothing more runs until \
             `decree resume` ({path})"
        ),
        Paused::Exists => {
            println!("already paused since {path} was written; `decree resume` to go on")
        }
    }
    Ok(())
}

/// Run `decree resume`: delete the file, or say there was none; name any other file that
/// still holds the queue.
pub fn resume(project_root: &Path, format: Format) -> Result<(), DecreeError> {
    let path = path();
    let resumed = attention::resume(project_root)
        .map_err(|e| DecreeError::Other(format!("cannot resume: {e}")))?;
    if format == Format::Json {
        return print_json(&json!({ "attention": path, "paused": false, "resumed": resumed }));
    }
    if resumed {
        println!("resumed: deleted {path}");
    } else {
        println!("not paused: {path} does not exist");
    }
    let holding: Vec<String> = attention::list(project_root)?
        .iter()
        .filter(|e| e.holds())
        .map(attention::Entry::summary)
        .collect();
    if !holding.is_empty() {
        println!("still held:\n  {}", holding.join("\n  "));
    }
    Ok(())
}
