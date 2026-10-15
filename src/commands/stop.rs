//! `decree stop [-m <note>] [--format <text|json>]` (docs/reference/cli.md): write
//! `.decree/attention/manual-stop.md`, which holds the queue (docs/reference/cli.md,
//! Attention): the runs going now finish their current step loop and no new run starts,
//! until a person deletes the file. A file already there is left as it is. decree writes
//! nothing else for it: no pid list, no signal.

use std::path::Path;

use serde_json::json;

use crate::attention::{self, Stopped, MANUAL_STOP_FILE};
use crate::cli::Format;
use crate::commands::print_json;
use crate::error::DecreeError;
use crate::layout::{ATTENTION_DIR, DECREE_DIR};

/// Run `decree stop`: write the file, or say it is there, and print its path.
pub fn run(project_root: &Path, note: Option<&str>, format: Format) -> Result<(), DecreeError> {
    let path = format!("{DECREE_DIR}/{ATTENTION_DIR}/{MANUAL_STOP_FILE}");
    let stopped = attention::stop(project_root, note, chrono::Utc::now())
        .map_err(|e| DecreeError::Other(format!("cannot write {path}: {e}")))?;
    if format == Format::Json {
        return print_json(&json!({ "attention": path }));
    }
    match stopped {
        Stopped::Written => {
            println!("stopping after the current run; delete {path} to resume")
        }
        Stopped::Exists => println!("already stopped: {path} exists; delete it to resume"),
    }
    Ok(())
}
