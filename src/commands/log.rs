use std::fs;
use std::path::Path;

use crate::error::{find_project_root, DecreeError};
use crate::message;

pub fn run(id: Option<&str>) -> Result<(), DecreeError> {
    let root = find_project_root()?;
    let runs_dir = root.join(".decree/runs");

    match id {
        None => {
            // Show the most recent message's log
            let latest = message::most_recent(&runs_dir)?;
            print_log(&runs_dir, &latest)?;
        }
        Some(prefix) => {
            let matches = message::resolve_id(&runs_dir, prefix)?;

            if matches.len() == 1 {
                print_log(&runs_dir, &matches[0])?;
            } else {
                // Check if all matches share the same chain (chain ID was given)
                let first_chain = matches[0].rsplit_once('-').map(|(c, _)| c);
                let all_same_chain = matches
                    .iter()
                    .all(|m| m.rsplit_once('-').map(|(c, _)| c) == first_chain);

                if all_same_chain {
                    // Show all logs in the chain
                    for m in &matches {
                        println!("--- {} ---", m);
                        print_log(&runs_dir, m)?;
                        println!();
                    }
                } else {
                    return Err(DecreeError::AmbiguousId {
                        prefix: prefix.to_string(),
                        candidates: matches,
                    });
                }
            }
        }
    }

    Ok(())
}

fn print_log(runs_dir: &Path, msg_id: &str) -> Result<(), DecreeError> {
    let msg_dir = runs_dir.join(msg_id);

    // Shell routine log
    let routine_log = msg_dir.join("routine.log");
    if routine_log.exists() {
        let content = fs::read_to_string(&routine_log)?;
        println!("{content}");
        return Ok(());
    }

    // Notebook routine output
    let output_ipynb = msg_dir.join("output.ipynb");
    let papermill_log = msg_dir.join("papermill.log");

    if output_ipynb.exists() || papermill_log.exists() {
        if papermill_log.exists() {
            let content = fs::read_to_string(&papermill_log)?;
            if !content.is_empty() {
                println!("=== Papermill Log ===");
                println!("{content}");
            }
        }
        if output_ipynb.exists() {
            println!("=== Notebook Output ===");
            println!("  {}", output_ipynb.display());
            // Print cell outputs from the notebook
            let nb_content = fs::read_to_string(&output_ipynb)?;
            if let Ok(nb) = serde_json::from_str::<serde_json::Value>(&nb_content) {
                if let Some(cells) = nb.get("cells").and_then(|c| c.as_array()) {
                    for (i, cell) in cells.iter().enumerate() {
                        if let Some(outputs) = cell.get("outputs").and_then(|o| o.as_array()) {
                            if outputs.is_empty() {
                                continue;
                            }
                            println!("\n  Cell {i}:");
                            for output in outputs {
                                if let Some(text) = output.get("text").and_then(|t| t.as_array()) {
                                    for line in text {
                                        if let Some(s) = line.as_str() {
                                            print!("    {s}");
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        return Ok(());
    }

    println!("No log files found for message {msg_id}");
    Ok(())
}
