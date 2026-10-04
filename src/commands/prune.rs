//! `decree prune --older-than <age> [--dry-run]` (docs/reference/cli.md): delete the folder
//! of every finished run whose `run_finished` event is older than `<age>`, a duration
//! (`crate::duration`). Nothing else ever
//! deletes a run (docs/decisions.md, D43); its history lives on in the log store
//! (docs/reference/observability.md).
//!
//! Kept, however old: a run that is not finished, a migration run whose file is not in
//! `processed.md` (deleting it would let `process` start the migration again), and a child
//! run whose parent still exists and is not finished. A run whose lock is held is skipped.

use std::collections::BTreeSet;
use std::io;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use colored::Colorize;

use serde_json::json;

use crate::cli::Format;
use crate::commands::check::{read_processed, Project};
use crate::commands::print_json;
use crate::commands::process::context;
use crate::error::DecreeError;
use crate::events::text;
use crate::interpreter::Context;
use crate::layout::MESSAGE_FILE;
use crate::message::{
    is_valid_id, lock_state, run_ids, LockState, Message, MessageError, RunLock, LOCK_FILE,
};

/// A finished run old enough to prune, as its `run_finished` event names it.
struct Finished {
    machine: String,
    state: String,
    ts: String,
}

/// Run `decree prune`.
pub fn run(
    project_root: &Path,
    older_than: Duration,
    dry_run: bool,
    format: Format,
) -> Result<(), DecreeError> {
    // `crate::duration::parse` keeps every duration within what `TimeDelta` holds.
    let older_than = TimeDelta::from_std(older_than).unwrap_or(TimeDelta::MAX);
    let project = Project::load(project_root)?;
    let ctx = context(project_root, &project, Arc::new(AtomicBool::new(false)));
    let processed = read_processed(&project.decree_dir)?;
    let rule = Rule {
        ctx: &ctx,
        processed: &processed,
        // An age reaching back past the earliest time chrono holds: no run is that old.
        cutoff: Utc::now()
            .checked_sub_signed(older_than)
            .unwrap_or(DateTime::<Utc>::MIN_UTC),
    };
    let verb = if dry_run { "would prune" } else { "pruned" };
    let (mut count, mut bytes, mut errors) = (0, 0, 0);
    let mut runs = Vec::new();
    for id in run_ids(&ctx.runs_dir())? {
        match prune(&rule, &id, dry_run) {
            Ok(Some((run, size))) => {
                match format {
                    Format::Text => println!(
                        "{verb} {id}  {}  {}  finished {}",
                        run.machine, run.state, run.ts
                    ),
                    Format::Json => runs.push(json!({
                        "id": id,
                        "machine": run.machine,
                        "state": run.state,
                        "finished": run.ts,
                    })),
                }
                count += 1;
                bytes += size;
            }
            Ok(None) => {}
            Err(e) => {
                eprintln!("{}: run {id}: {e}", "error".red());
                errors += 1;
            }
        }
    }
    let size = human_size(bytes);
    match format {
        Format::Text if dry_run => println!("would prune {count} run(s), {size}"),
        Format::Text => println!("pruned {count} run(s), {size} freed"),
        Format::Json => print_json(&json!({ "runs": runs, "bytes": bytes, "dry_run": dry_run }))?,
    }
    match errors {
        0 => Ok(()),
        n => Err(DecreeError::Other(format!(
            "{n} run(s) could not be pruned"
        ))),
    }
}

/// What makes a run prunable: finished before `cutoff`, and none of the cases kept.
struct Rule<'a> {
    ctx: &'a Context<'a>,
    processed: &'a BTreeSet<String>,
    cutoff: DateTime<Utc>,
}

impl Rule<'_> {
    /// Run `id`'s `run_finished` event, if the run may be pruned.
    fn check(&self, id: &str) -> Result<Option<Finished>, DecreeError> {
        let Some(last) = self.ctx.run_finished(id)? else {
            return Ok(None);
        };
        let field = |key| text(&last, key).unwrap_or_default().to_string();
        let ts = field("ts");
        let at = DateTime::parse_from_rfc3339(&ts).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("`run_finished` has no valid `ts` ({e})"),
            )
        })?;
        if at > self.cutoff {
            return Ok(None);
        }
        // A migration that ended in `failed` is not in the ledger; its folder is what keeps
        // `process` from starting it again (docs/reference/messages.md, Migrations).
        if field("trigger") == "migration" && !self.processed.contains(&format!("{id}.md")) {
            return Ok(None);
        }
        // The parent may still read its child's results (docs/reference/runs.md,
        // Sub-machines).
        if let Some(parent) = self.parent(id)? {
            let alive = self.ctx.runs_dir().join(&parent).is_dir()
                && self.ctx.run_finished(&parent)?.is_none();
            if alive {
                return Ok(None);
            }
        }
        Ok(Some(Finished {
            machine: field("machine"),
            state: field("state"),
            ts,
        }))
    }

    /// The `parent` in run `id`'s `message.md`, if it names a run. A frontmatter that does
    /// not parse names none.
    fn parent(&self, id: &str) -> Result<Option<String>, DecreeError> {
        let path = self.ctx.runs_dir().join(id).join(MESSAGE_FILE);
        let message = match Message::read(&path) {
            Ok(message) => message,
            Err(MessageError::Parse { .. }) => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        Ok(message
            .text("parent")
            .filter(|p| is_valid_id(p))
            .map(String::from))
    }
}

/// Prune run `id` if `rule` allows it, or with `dry_run` only say so. Returns the run and
/// the bytes of its files. A run whose lock a live process holds is skipped.
fn prune(rule: &Rule, id: &str, dry_run: bool) -> Result<Option<(Finished, u64)>, DecreeError> {
    let Some(run) = rule.check(id)? else {
        return Ok(None);
    };
    let run_dir = rule.ctx.runs_dir().join(id);
    // A dry run takes no lock, so it writes nothing; it skips the runs a prune would.
    if dry_run {
        if matches!(lock_state(&run_dir)?, LockState::Live(_)) {
            return Ok(None);
        }
        return Ok(Some((run, dir_size(&run_dir)?)));
    }
    let Some(lock) = RunLock::acquire(&run_dir)? else {
        return Ok(None);
    };
    // Again under the lock: `decree process --retry` may have appended after `run_finished`.
    let Some(run) = rule.check(id)? else {
        return Ok(None);
    };
    let size = dir_size(&run_dir)?;
    remove_run(&run_dir, lock)?;
    Ok(Some((run, size)))
}

/// Delete `run_dir`, whose lock this process holds as `lock`. Everything but `.lock` goes
/// first, so no other process can take the run while it is half deleted; dropping `lock`
/// then deletes `.lock` (it holds no open file), and the empty folder goes last.
fn remove_run(run_dir: &Path, lock: RunLock) -> io::Result<()> {
    for entry in std::fs::read_dir(run_dir)? {
        let entry = entry?;
        if entry.file_name() == LOCK_FILE {
            continue;
        }
        if entry.file_type()?.is_dir() {
            std::fs::remove_dir_all(entry.path())?;
        } else {
            std::fs::remove_file(entry.path())?;
        }
    }
    drop(lock);
    std::fs::remove_dir(run_dir)
}

/// The bytes of the files under `dir`, without following symlinks and without the run's
/// own `.lock`, so a dry run and a prune report the same size.
fn dir_size(dir: &Path) -> io::Result<u64> {
    let mut total = 0;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let meta = entry.path().symlink_metadata()?;
        if meta.is_dir() {
            total += dir_size(&entry.path())?;
        } else if entry.file_name() != LOCK_FILE {
            total += meta.len();
        }
    }
    Ok(total)
}

/// `bytes` in KB, MB or GB (powers of 1024, as `du -h`): whole KB below 1 MB, rounded up,
/// else one decimal.
fn human_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * KB;
    const GB: u64 = MB * KB;
    if bytes < MB {
        format!("{} KB", bytes.div_ceil(KB))
    } else if bytes < GB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_human_size_uses_kb_mb_and_gb() {
        assert_eq!(human_size(0), "0 KB");
        assert_eq!(human_size(1), "1 KB");
        assert_eq!(human_size(64 * 1024), "64 KB");
        assert_eq!(human_size(1024 * 1024 * 3 / 2), "1.5 MB");
        assert_eq!(human_size(1024 * 1024 * 1024 * 2), "2.0 GB");
    }
}
