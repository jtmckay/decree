//! `decree schema`: write the JSON Schemas (draft 2020-12) for machines and message
//! frontmatter into `.decree/schema/` (docs/reference/machines.md, Schema), each through a temp
//! file and a rename. `decree check` uses `stale` to warn when they are missing or out of date.
//! The schemas describe shape only; `decree check` stays the authority for meaning.

use std::path::Path;

use crate::error::DecreeError;
use crate::layout::DECREE_DIR;
use crate::message::write_replace;

/// Directory under `.decree/` that `decree schema` writes.
pub const SCHEMA_DIR: &str = "schema";

/// Every schema `decree schema` writes: filename and content, compiled into the binary from
/// `src/templates/schema/`, their single source.
const SCHEMAS: [(&str, &str); 2] = [
    (
        "machine.schema.json",
        include_str!("../templates/schema/machine.schema.json"),
    ),
    (
        "message.schema.json",
        include_str!("../templates/schema/message.schema.json"),
    ),
];

pub fn run(project_root: &Path) -> Result<(), DecreeError> {
    for name in write(project_root)? {
        println!("{DECREE_DIR}/{SCHEMA_DIR}/{name}");
    }
    Ok(())
}

/// Write every schema into `.decree/schema/`, each through `.<name>.tmp` and a rename.
/// Returns the filenames written, in name order.
pub fn write(project_root: &Path) -> Result<Vec<String>, DecreeError> {
    let dir = project_root.join(DECREE_DIR).join(SCHEMA_DIR);
    std::fs::create_dir_all(&dir)?;
    for (name, text) in SCHEMAS {
        write_replace(&dir.join(name), text.as_bytes()).map_err(|(path, e)| {
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
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both schemas are JSON with an `$id` and a `title`, and name draft 2020-12.
    #[test]
    fn schemas_are_json_with_an_id_and_a_title() {
        for (name, text) in SCHEMAS {
            let schema: serde_json::Value = serde_json::from_str(text).unwrap();
            assert_eq!(
                schema["$schema"], "https://json-schema.org/draft/2020-12/schema",
                "{name}"
            );
            assert!(
                schema["$id"].as_str().is_some_and(|id| id.ends_with(name)),
                "{name}"
            );
            assert!(schema["title"].is_string(), "{name}");
        }
    }

    #[test]
    fn write_then_stale_is_empty_and_leaves_no_temp_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        assert_eq!(
            stale(tmp.path()).unwrap(),
            [
                "schema/machine.schema.json: missing",
                "schema/message.schema.json: missing"
            ]
        );
        assert_eq!(
            write(tmp.path()).unwrap(),
            ["machine.schema.json", "message.schema.json"]
        );
        assert!(stale(tmp.path()).unwrap().is_empty());
        let dir = tmp.path().join(DECREE_DIR).join(SCHEMA_DIR);
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["machine.schema.json", "message.schema.json"]);

        std::fs::write(dir.join("message.schema.json"), "{}\n").unwrap();
        assert_eq!(
            stale(tmp.path()).unwrap(),
            ["schema/message.schema.json: out of date"]
        );
    }
}
