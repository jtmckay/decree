use crate::diff_apply;
use crate::error::{find_project_root, DecreeError};
use crate::message;

pub fn run(id: Option<&str>, since: Option<&str>) -> Result<(), DecreeError> {
    let root = find_project_root()?;
    let runs_dir = root.join(".decree/runs");

    let msg_ids = resolve_diff_targets(&runs_dir, id, since)?;

    let mut any_output = false;
    for msg_id in &msg_ids {
        match diff_apply::read_diff(&runs_dir, msg_id)? {
            Some(content) => {
                print!("{content}");
                any_output = true;
            }
            None => {
                if msg_ids.len() == 1 {
                    println!("No changes.diff for message {msg_id}.");
                    println!("The message may have had no changes or is still in progress.");
                }
            }
        }
    }

    if !any_output && msg_ids.len() > 1 {
        println!("No changes found in the specified messages.");
    }

    Ok(())
}

/// Resolve the diff command targets based on arguments.
fn resolve_diff_targets(
    runs_dir: &std::path::Path,
    id: Option<&str>,
    since: Option<&str>,
) -> Result<Vec<String>, DecreeError> {
    if let Some(since_id) = since {
        // --since <id>: from this message onward
        return diff_apply::messages_since(runs_dir, since_id);
    }

    match id {
        None => {
            // No args: most recent message
            let latest = message::most_recent(runs_dir)?;
            Ok(vec![latest])
        }
        Some(prefix) => {
            // Could be a full message ID or a chain ID
            diff_apply::resolve_targets(runs_dir, prefix)
        }
    }
}
