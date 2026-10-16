//! Which runs are a migration's (docs/reference/messages.md, Migrations, rule 1). A
//! migration's run has a time id like every other run, and its `message.md` records the
//! file it came from as `migration: <file>`. A run from before that, named by the file's
//! stem, is still that migration's run: run folders are never renamed.

use std::collections::BTreeMap;
use std::io;
use std::path::Path;

use crate::layout::MESSAGE_FILE;
use crate::message::{is_message_id, run_ids, Message};

/// The frontmatter key, in `runs/<id>/message.md`, that names the migration file a run is
/// a run of. Only decree sets it.
pub const MIGRATION_KEY: &str = "migration";

/// The migration file run `id` is a run of: its `message.md`'s `migration`, or `<id>.md` for
/// a run from before migration runs had time ids, named by the file's stem with
/// `trigger: migration`. `None` for any other run, and for one whose `message.md` cannot be
/// read.
pub fn of_run(runs_dir: &Path, id: &str) -> Option<String> {
    let message = Message::read(&runs_dir.join(id).join(MESSAGE_FILE)).ok()?;
    if let Some(file) = message.text(MIGRATION_KEY) {
        return Some(file.to_string());
    }
    (message.text("trigger") == Some("migration") && !is_message_id(id)).then(|| format!("{id}.md"))
}

/// Each migration file that has a run, and its newest run: the latest time id, else the
/// folder named by its stem.
pub fn newest_runs(runs_dir: &Path) -> io::Result<BTreeMap<String, String>> {
    let mut newest = BTreeMap::new();
    for id in run_ids(runs_dir)? {
        let Some(file) = of_run(runs_dir, &id) else {
            continue;
        };
        if is_message_id(&id) {
            // `run_ids` is in id order, so a later time id is a newer run.
            newest.insert(file, id);
        } else {
            newest.entry(file).or_insert(id);
        }
    }
    Ok(newest)
}

/// Migration `file`'s newest run (`newest_runs`), if it has one.
pub fn newest_run(runs_dir: &Path, file: &str) -> io::Result<Option<String>> {
    Ok(newest_runs(runs_dir)?.remove(file))
}

/// The run a person means by `name` (docs/reference/cli.md): a migration's file name or
/// stem means that migration's newest run; else `name` is a run id. `None` when neither
/// names a run.
pub fn resolve(runs_dir: &Path, name: &str) -> io::Result<Option<String>> {
    if name.is_empty() || name.contains('/') {
        return Ok(None);
    }
    let file = if name.ends_with(".md") {
        name.to_string()
    } else {
        format!("{name}.md")
    };
    if let Some(run) = newest_run(runs_dir, &file)? {
        return Ok(Some(run));
    }
    Ok(runs_dir.join(name).is_dir().then(|| name.to_string()))
}

/// How a person sees run `id`: `<id> (<file>)` for a migration's run, else `<id>`.
pub fn label(runs_dir: &Path, id: &str) -> String {
    match of_run(runs_dir, id) {
        Some(file) => format!("{id} ({file})"),
        None => id.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn run(dir: &TempDir, id: &str, frontmatter: &str) {
        let run_dir = dir.path().join(id);
        std::fs::create_dir_all(&run_dir).unwrap();
        std::fs::write(
            run_dir.join(MESSAGE_FILE),
            format!("---\n{frontmatter}---\n"),
        )
        .unwrap();
    }

    const OLD: &str = "20261001T143005.218734Z-3fa9c1";
    const NEW: &str = "20261002T143005.218734Z-3fa9c1";

    #[test]
    fn of_run_reads_the_migration_key_or_a_stem_named_migration_run() {
        let dir = TempDir::new().unwrap();
        run(&dir, NEW, "migration: 01-a.md\ntrigger: migration\n");
        run(&dir, "02-b", "id: 02-b\ntrigger: migration\n");
        run(&dir, "03-c", "id: 03-c\ntrigger: inbox\n");
        run(&dir, OLD, "trigger: migration\n");
        assert_eq!(of_run(dir.path(), NEW).as_deref(), Some("01-a.md"));
        assert_eq!(of_run(dir.path(), "02-b").as_deref(), Some("02-b.md"));
        assert_eq!(of_run(dir.path(), "03-c"), None);
        assert_eq!(of_run(dir.path(), OLD), None, "a time id is never a stem");
        assert_eq!(of_run(dir.path(), "missing"), None);
    }

    #[test]
    fn newest_runs_prefers_the_latest_time_id_over_a_stem_named_run() {
        let dir = TempDir::new().unwrap();
        run(&dir, "01-a", "trigger: migration\n");
        run(&dir, OLD, "migration: 01-a.md\n");
        run(&dir, NEW, "migration: 01-a.md\n");
        // A stem that sorts after the time ids.
        run(&dir, "90-z", "trigger: migration\n");
        run(
            &dir,
            "20261003T143005.218734Z-3fa9c1",
            "migration: 90-z.md\n",
        );
        let newest = newest_runs(dir.path()).unwrap();
        assert_eq!(newest["01-a.md"], NEW);
        assert_eq!(newest["90-z.md"], "20261003T143005.218734Z-3fa9c1");
        assert!(newest_runs(&dir.path().join("none")).unwrap().is_empty());
    }

    #[test]
    fn resolve_takes_a_file_name_a_stem_or_a_run_id() {
        let dir = TempDir::new().unwrap();
        run(&dir, NEW, "migration: 01-a.md\n");
        run(&dir, "02-b", "trigger: migration\n");
        run(&dir, OLD, "trigger: emit\n");
        for name in ["01-a", "01-a.md", NEW] {
            assert_eq!(resolve(dir.path(), name).unwrap().as_deref(), Some(NEW));
        }
        assert_eq!(
            resolve(dir.path(), "02-b").unwrap().as_deref(),
            Some("02-b")
        );
        assert_eq!(resolve(dir.path(), OLD).unwrap().as_deref(), Some(OLD));
        for name in ["03-c", "", "../x", "01-a.md/x"] {
            assert_eq!(resolve(dir.path(), name).unwrap(), None, "{name}");
        }
    }

    #[test]
    fn label_names_a_migration_runs_file() {
        let dir = TempDir::new().unwrap();
        run(&dir, NEW, "migration: 01-a.md\n");
        run(&dir, OLD, "trigger: emit\n");
        assert_eq!(label(dir.path(), NEW), format!("{NEW} (01-a.md)"));
        assert_eq!(label(dir.path(), OLD), OLD);
    }
}
