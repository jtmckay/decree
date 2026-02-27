use std::io::{self, Write};

use crate::diff_apply::{self, DiffStats};
use crate::error::{find_project_root, DecreeError};
use crate::message;

pub fn run(
    id: Option<String>,
    through: Option<String>,
    since: Option<String>,
    all: bool,
    force: bool,
) -> Result<(), DecreeError> {
    let root = find_project_root()?;
    let runs_dir = root.join(".decree/runs");

    // No arguments: list messages
    if id.is_none() && through.is_none() && since.is_none() && !all {
        return list_messages(&runs_dir);
    }

    // Resolve targets
    let msg_ids = resolve_apply_targets(&runs_dir, id.as_deref(), through.as_deref(), since.as_deref(), all)?;

    // Gather diffs
    let mut apply_items: Vec<(String, String)> = Vec::new(); // (msg_id, diff_content)
    for msg_id in &msg_ids {
        match diff_apply::read_diff(&runs_dir, msg_id)? {
            Some(content) => apply_items.push((msg_id.clone(), content)),
            None => {
                eprintln!("Skipping {msg_id}: no changes.diff");
            }
        }
    }

    if apply_items.is_empty() {
        println!("No changes to apply.");
        return Ok(());
    }

    // Parse all diffs
    let mut all_parsed = Vec::new();
    for (msg_id, diff_content) in &apply_items {
        let file_diffs = diff_apply::parse_diff(diff_content)?;
        all_parsed.push((msg_id.as_str(), diff_content.as_str(), file_diffs));
    }

    if force {
        // Force mode: warn and confirm
        eprint!("WARNING: --force will overwrite conflicting files. Continue? [y/N] ");
        io::stderr().flush().ok();
        let mut answer = String::new();
        io::stdin().read_line(&mut answer)?;
        if !answer.trim().eq_ignore_ascii_case("y") {
            println!("Aborted.");
            return Ok(());
        }

        for (msg_id, _, file_diffs) in &all_parsed {
            diff_apply::apply_diffs(&root, file_diffs)?;
            eprintln!("Applied {msg_id}");
        }
        return Ok(());
    }

    // Pre-apply conflict check for each message
    for (msg_id, _, file_diffs) in &all_parsed {
        let conflicts = diff_apply::check_conflicts(&root, file_diffs);
        if !conflicts.is_empty() {
            print_conflict_report(msg_id, &conflicts);
            return Ok(());
        }
    }

    // Show confirmation summary
    print_apply_summary(&apply_items)?;
    eprint!("Proceed? [Y/n] ");
    io::stderr().flush().ok();
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    let answer = answer.trim();
    if !answer.is_empty() && !answer.eq_ignore_ascii_case("y") {
        println!("Aborted.");
        return Ok(());
    }

    // Apply
    for (msg_id, _, file_diffs) in &all_parsed {
        diff_apply::apply_diffs(&root, file_diffs)?;
        eprintln!("Applied {msg_id}");
    }

    Ok(())
}

/// Resolve apply targets based on arguments.
fn resolve_apply_targets(
    runs_dir: &std::path::Path,
    id: Option<&str>,
    through: Option<&str>,
    since: Option<&str>,
    all: bool,
) -> Result<Vec<String>, DecreeError> {
    if all {
        return diff_apply::all_messages(runs_dir);
    }

    if let Some(through_id) = through {
        return diff_apply::messages_through(runs_dir, through_id);
    }

    if let Some(since_id) = since {
        return diff_apply::messages_since(runs_dir, since_id);
    }

    match id {
        Some(prefix) => diff_apply::resolve_targets(runs_dir, prefix),
        None => {
            // Should not reach here (handled above), but just in case
            let latest = message::most_recent(runs_dir)?;
            Ok(vec![latest])
        }
    }
}

/// Print the listing of all messages grouped by chain.
fn list_messages(runs_dir: &std::path::Path) -> Result<(), DecreeError> {
    let chains = diff_apply::list_messages(runs_dir)?;

    if chains.is_empty() {
        println!("No messages found.");
        return Ok(());
    }

    println!("Messages:\n");

    for (_chain, messages) in &chains {
        if messages.is_empty() {
            continue;
        }

        // Chain header: chain ID + first message description
        let first = &messages[0];
        let desc = if first.description.is_empty() {
            String::new()
        } else {
            format!("  {}", first.description)
        };
        println!("  {}{}", first.chain, desc);

        // Individual messages
        for msg in messages {
            let files_label = if msg.stats.files == 1 {
                "1 file".to_string()
            } else {
                format!("{} files", msg.stats.files)
            };

            let task_desc = if msg.description.starts_with("task:") {
                format!("   {}", msg.description)
            } else {
                String::new()
            };

            println!(
                "    {}  +{:<3} -{:<3} ({}){task_desc}",
                msg.seq, msg.stats.additions, msg.stats.deletions, files_label,
            );
        }

        println!();
    }

    Ok(())
}

/// Print conflict report.
fn print_conflict_report(msg_id: &str, conflicts: &[diff_apply::Conflict]) {
    eprintln!(
        "Cannot apply changes from {msg_id} -- conflicts detected:\n"
    );

    // Group by file
    let mut by_file: std::collections::BTreeMap<&str, Vec<&str>> =
        std::collections::BTreeMap::new();
    for c in conflicts {
        by_file.entry(&c.file).or_default().push(&c.detail);
    }

    for (file, details) in &by_file {
        eprintln!("  {file}:");
        for detail in details {
            eprintln!("    {detail}");
        }
    }

    eprintln!("\nAborting. No files were modified.");
    eprintln!("Hint: use `decree diff {msg_id}` to inspect the changes.");
}

/// Print confirmation summary before applying.
fn print_apply_summary(items: &[(String, String)]) -> Result<(), DecreeError> {
    if items.len() == 1 {
        let stats = diff_apply::compute_stats(&items[0].1);
        let files_label = if stats.files == 1 {
            "1 file".to_string()
        } else {
            format!("{} files", stats.files)
        };
        eprintln!(
            "Will apply {} -- +{} -{} ({})\n",
            items[0].0, stats.additions, stats.deletions, files_label
        );
    } else {
        // Determine if all items share the same chain
        let first_chain = items[0].0.rsplit_once('-').map(|(c, _)| c);
        let all_same = items
            .iter()
            .all(|(id, _)| id.rsplit_once('-').map(|(c, _)| c) == first_chain);

        if all_same {
            if let Some(chain) = first_chain {
                eprintln!(
                    "Will apply chain {chain} ({} messages):\n",
                    items.len()
                );
            }
        } else {
            eprintln!("Will apply {} messages:\n", items.len());
        }

        let mut total = DiffStats::default();
        let mut total_files = std::collections::HashSet::new();

        for (msg_id, diff_content) in items {
            let stats = diff_apply::compute_stats(diff_content);
            let seq = msg_id.rsplit_once('-').map(|(_, s)| s).unwrap_or(msg_id);
            let files_label = if stats.files == 1 {
                "1 file".to_string()
            } else {
                format!("{} files", stats.files)
            };
            eprintln!(
                "  -{seq}  +{:<3} -{:<3} ({files_label})",
                stats.additions, stats.deletions,
            );
            total.additions += stats.additions;
            total.deletions += stats.deletions;
            // Approximate total unique files
            total_files.insert(msg_id.clone());
        }

        let total_files_count: usize = items
            .iter()
            .map(|(_, d)| diff_apply::compute_stats(d).files)
            .sum();
        eprintln!(
            "\n  Total: +{} -{} ({} files)\n",
            total.additions, total.deletions, total_files_count,
        );
    }

    Ok(())
}
