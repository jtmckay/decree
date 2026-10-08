//! `decree schema`: write the JSON Schemas (draft 2020-12) of every file decree reads or
//! writes, and of every document a command prints with `--format json` (in `cli/`), into
//! `.decree/schema/v1/` (docs/reference/README.md, Schemas), each through a temp
//! file and a rename, and remove anything else in `.decree/schema/`. The local copy is
//! optional (editors read the hosted copy in `schema/`, docs/editors.md); when it exists,
//! `decree check` uses `stale_files` to warn when one is missing, out of date, or joined by
//! files decree does not write. The schemas describe shape only; `decree check` stays the authority for meaning.

use std::path::Path;

use crate::cli::Format;
use crate::commands::print_json;
use crate::error::DecreeError;
use crate::layout::DECREE_DIR;
use crate::message::write_replace;

/// Directory under `.decree/` that `decree schema` writes.
pub const SCHEMA_DIR: &str = "schema";

/// Every schema `decree schema` writes: its path under `.decree/schema/` and its content,
/// compiled into the binary from `schema/` at the repository root, their single source and the published copy. The first
/// path segment is the contract version (docs/reference/README.md, Versioning); `cli/` holds
/// the documents commands print with `--format json` (docs/reference/cli.md).
const SCHEMAS: [(&str, &str); 14] = [
    (
        "v1/cli/check.schema.json",
        include_str!("../../schema/v1/cli/check.schema.json"),
    ),
    (
        "v1/cli/emit.schema.json",
        include_str!("../../schema/v1/cli/emit.schema.json"),
    ),
    (
        "v1/cli/event.schema.json",
        include_str!("../../schema/v1/cli/event.schema.json"),
    ),
    (
        "v1/cli/graph.schema.json",
        include_str!("../../schema/v1/cli/graph.schema.json"),
    ),
    (
        "v1/cli/process.schema.json",
        include_str!("../../schema/v1/cli/process.schema.json"),
    ),
    (
        "v1/cli/prune.schema.json",
        include_str!("../../schema/v1/cli/prune.schema.json"),
    ),
    (
        "v1/cli/schema.schema.json",
        include_str!("../../schema/v1/cli/schema.schema.json"),
    ),
    (
        "v1/cli/skill.schema.json",
        include_str!("../../schema/v1/cli/skill.schema.json"),
    ),
    (
        "v1/cli/status.schema.json",
        include_str!("../../schema/v1/cli/status.schema.json"),
    ),
    (
        "v1/events.schema.json",
        include_str!("../../schema/v1/events.schema.json"),
    ),
    (
        "v1/machine.schema.json",
        include_str!("../../schema/v1/machine.schema.json"),
    ),
    (
        "v1/message.schema.json",
        include_str!("../../schema/v1/message.schema.json"),
    ),
    (
        "v1/reply.schema.json",
        include_str!("../../schema/v1/reply.schema.json"),
    ),
    (
        "v1/request.schema.json",
        include_str!("../../schema/v1/request.schema.json"),
    ),
];

pub fn run(project_root: &Path, format: Format) -> Result<(), DecreeError> {
    let removed = others(&project_root.join(DECREE_DIR).join(SCHEMA_DIR))?;
    let written = write(project_root)?;
    let path = |name: &String| format!("{DECREE_DIR}/{SCHEMA_DIR}/{name}");
    match format {
        Format::Text => {
            for name in &written {
                println!("{}", path(name));
            }
            Ok(())
        }
        Format::Json => print_json(&serde_json::json!({
            "written": written.iter().map(path).collect::<Vec<_>>(),
            "removed": removed.iter().map(path).collect::<Vec<_>>(),
        })),
    }
}

/// Write every schema into `.decree/schema/`, each through `.<name>.tmp` and a rename, and
/// remove every other file there, and the directories left empty. Returns the paths written,
/// relative to `.decree/schema/`, in order.
pub fn write(project_root: &Path) -> Result<Vec<String>, DecreeError> {
    let dir = project_root.join(DECREE_DIR).join(SCHEMA_DIR);
    for name in others(&dir)? {
        std::fs::remove_file(dir.join(&name))?;
    }
    remove_empty_dirs(&dir)?;
    for (name, text) in SCHEMAS {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        write_replace(&path, text.as_bytes()).map_err(|(path, e)| {
            DecreeError::Other(format!("cannot write {}: {e}", path.display()))
        })?;
    }
    Ok(SCHEMAS.iter().map(|(name, _)| name.to_string()).collect())
}

/// How `.decree/schema/` differs from what `decree schema` would write, one line per file,
/// each naming the file relative to `.decree/`, as `decree check` warns. Empty when it is
/// up to date.
#[cfg(test)]
fn stale(project_root: &Path) -> Result<Vec<String>, DecreeError> {
    Ok(stale_files(project_root)?
        .into_iter()
        .map(|(file, message)| format!("{file}: {message}"))
        .collect())
}

/// How `.decree/schema/` differs from what `decree schema` would write, one (file relative
/// to `.decree/`, what is wrong) pair per file. Empty when it is up to date, and when there
/// is no `.decree/schema/`: the local copy is optional, editors read the hosted schemas.
pub fn stale_files(project_root: &Path) -> Result<Vec<(String, String)>, DecreeError> {
    let dir = project_root.join(DECREE_DIR).join(SCHEMA_DIR);
    let mut out = Vec::new();
    if !dir.exists() {
        return Ok(out);
    }
    for (name, text) in SCHEMAS {
        match std::fs::read_to_string(dir.join(name)) {
            Ok(on_disk) if on_disk == text => {}
            Ok(_) => out.push((format!("{SCHEMA_DIR}/{name}"), "out of date".to_string())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                out.push((format!("{SCHEMA_DIR}/{name}"), "missing".to_string()))
            }
            Err(e) => return Err(e.into()),
        }
    }
    for name in others(&dir)? {
        out.push((
            format!("{SCHEMA_DIR}/{name}"),
            "not one decree writes".to_string(),
        ));
    }
    Ok(out)
}

/// Every file under `dir` that is not one of `SCHEMAS`, as a `/`-separated path relative to
/// `dir`, sorted. None if `dir` does not exist.
fn others(dir: &Path) -> Result<Vec<String>, DecreeError> {
    let mut out = Vec::new();
    let mut pending = vec![String::new()];
    while let Some(rel) = pending.pop() {
        let entries = match std::fs::read_dir(dir.join(&rel)) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        };
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = if rel.is_empty() {
                name
            } else {
                format!("{rel}/{name}")
            };
            if entry.file_type()?.is_dir() {
                pending.push(path);
            } else if !SCHEMAS.iter().any(|(known, _)| *known == path) {
                out.push(path);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Remove the empty directories below `dir`, deepest first; `dir` itself stays.
fn remove_empty_dirs(dir: &Path) -> Result<(), DecreeError> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    for entry in entries {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            let path = entry.path();
            remove_empty_dirs(&path)?;
            if std::fs::read_dir(&path)?.next().is_none() {
                std::fs::remove_dir(&path)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every schema is JSON with an `$id` naming its path and a `title`, and names draft
    /// 2020-12.
    #[test]
    fn schemas_are_json_with_an_id_and_a_title() {
        for (name, text) in SCHEMAS {
            let schema: serde_json::Value = serde_json::from_str(text).unwrap();
            assert_eq!(
                schema["$schema"], "https://json-schema.org/draft/2020-12/schema",
                "{name}"
            );
            assert_eq!(
                schema["$id"],
                format!("https://raw.githubusercontent.com/jtmckay/decree/main/schema/{name}"),
                "{name}"
            );
            assert!(schema["title"].is_string(), "{name}");
        }
    }

    /// The schemas compiled into decree are the bytes of `schema/` at the repository root,
    /// the copy published at each `$id`, and nothing there is left out.
    #[test]
    fn embedded_schemas_are_the_repository_schema_folder() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(SCHEMA_DIR);
        for (name, text) in SCHEMAS {
            let on_disk = std::fs::read_to_string(root.join(name)).unwrap();
            assert!(on_disk == text, "{name} differs from schema/{name}");
        }
        assert!(others(&root).unwrap().is_empty(), "{:?}", others(&root));
    }

    fn files(dir: &Path) -> Vec<String> {
        let mut out = others(dir).unwrap();
        out.extend(
            SCHEMAS
                .iter()
                .map(|(name, _)| name.to_string())
                .filter(|name| dir.join(name).is_file()),
        );
        out.sort();
        out
    }

    #[test]
    fn write_then_stale_is_empty_and_leaves_no_temp_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let names: Vec<String> = SCHEMAS.iter().map(|(n, _)| n.to_string()).collect();
        let missing: Vec<String> = names
            .iter()
            .map(|n| format!("schema/{n}: missing"))
            .collect();
        // No `.decree/schema/`: the local copy is optional, nothing to warn about.
        assert!(stale(tmp.path()).unwrap().is_empty());
        let dir = tmp.path().join(DECREE_DIR).join(SCHEMA_DIR);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(stale(tmp.path()).unwrap(), missing);
        assert_eq!(write(tmp.path()).unwrap(), names);
        assert!(stale(tmp.path()).unwrap().is_empty());
        assert_eq!(files(&dir), names);

        std::fs::write(dir.join("v1/message.schema.json"), "{}\n").unwrap();
        assert_eq!(
            stale(tmp.path()).unwrap(),
            ["schema/v1/message.schema.json: out of date"]
        );
    }

    /// The unversioned files of decree 0.5 before migration 81, and anything else, are
    /// reported, then removed with the directories they leave empty.
    #[test]
    fn other_files_are_stale_until_write_removes_them() {
        let tmp = tempfile::TempDir::new().unwrap();
        write(tmp.path()).unwrap();
        let dir = tmp.path().join(DECREE_DIR).join(SCHEMA_DIR);
        std::fs::write(dir.join("machine.schema.json"), "{}\n").unwrap();
        std::fs::create_dir_all(dir.join("v0/old")).unwrap();
        std::fs::write(dir.join("v0/old/x.json"), "{}\n").unwrap();
        assert_eq!(
            stale(tmp.path()).unwrap(),
            [
                "schema/machine.schema.json: not one decree writes",
                "schema/v0/old/x.json: not one decree writes"
            ]
        );
        write(tmp.path()).unwrap();
        assert!(stale(tmp.path()).unwrap().is_empty());
        assert!(!dir.join("v0").exists());
        assert!(dir.join("v1").is_dir());
    }
}
