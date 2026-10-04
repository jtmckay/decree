//! `decree schema`: write the JSON Schemas (draft 2020-12) of every file decree reads or
//! writes into `.decree/schema/v1/` (docs/reference/README.md, Schemas), each through a temp
//! file and a rename, and remove anything else in `.decree/schema/`. `decree check` uses
//! `stale` to warn when they are missing, out of date, or joined by files decree does not
//! write. The schemas describe shape only; `decree check` stays the authority for meaning.

use std::path::Path;

use crate::error::DecreeError;
use crate::layout::DECREE_DIR;
use crate::message::write_replace;

/// Directory under `.decree/` that `decree schema` writes.
pub const SCHEMA_DIR: &str = "schema";

/// Every schema `decree schema` writes: its path under `.decree/schema/` and its content,
/// compiled into the binary from `src/templates/schema/`, their single source. The first
/// path segment is the contract version (docs/reference/README.md, Versioning).
const SCHEMAS: [(&str, &str); 5] = [
    (
        "v1/events.schema.json",
        include_str!("../templates/schema/v1/events.schema.json"),
    ),
    (
        "v1/machine.schema.json",
        include_str!("../templates/schema/v1/machine.schema.json"),
    ),
    (
        "v1/message.schema.json",
        include_str!("../templates/schema/v1/message.schema.json"),
    ),
    (
        "v1/reply.schema.json",
        include_str!("../templates/schema/v1/reply.schema.json"),
    ),
    (
        "v1/request.schema.json",
        include_str!("../templates/schema/v1/request.schema.json"),
    ),
];

pub fn run(project_root: &Path) -> Result<(), DecreeError> {
    for name in write(project_root)? {
        println!("{DECREE_DIR}/{SCHEMA_DIR}/{name}");
    }
    Ok(())
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
/// each naming the file relative to `.decree/`. Empty when it is up to date.
pub fn stale(project_root: &Path) -> Result<Vec<String>, DecreeError> {
    let dir = project_root.join(DECREE_DIR).join(SCHEMA_DIR);
    let mut out = Vec::new();
    for (name, text) in SCHEMAS {
        match std::fs::read_to_string(dir.join(name)) {
            Ok(on_disk) if on_disk == text => {}
            Ok(_) => out.push(format!("{SCHEMA_DIR}/{name}: out of date")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                out.push(format!("{SCHEMA_DIR}/{name}: missing"))
            }
            Err(e) => return Err(e.into()),
        }
    }
    for name in others(&dir)? {
        out.push(format!("{SCHEMA_DIR}/{name}: not one decree writes"));
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
                format!(
                    "https://raw.githubusercontent.com/jtmckay/decree/main/src/templates/schema/{name}"
                ),
                "{name}"
            );
            assert!(schema["title"].is_string(), "{name}");
        }
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
        assert_eq!(stale(tmp.path()).unwrap(), missing);
        assert_eq!(write(tmp.path()).unwrap(), names);
        assert!(stale(tmp.path()).unwrap().is_empty());
        let dir = tmp.path().join(DECREE_DIR).join(SCHEMA_DIR);
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
