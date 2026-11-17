//! Helpers shared by the test files (each `mod common;`), per the Rust Book's
//! `tests/common/mod.rs` convention.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

/// Write `text` to `path` as an executable script (mode 755), from a `sh` child process.
///
/// Tests run on parallel threads. Had this process written the file, a process another
/// thread forks at that moment would inherit the open write handle until it execs, and
/// running the script meanwhile fails with ETXTBSY ("Text file busy").
#[allow(dead_code)]
pub fn write_script(path: &Path, text: &str) {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    let mut sh = Command::new("sh")
        .args(["-c", r#"cat > "$1" && chmod 755 "$1""#, "sh"])
        .arg(path)
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    sh.stdin.take().unwrap().write_all(text.as_bytes()).unwrap();
    assert!(sh.wait().unwrap().success(), "{}", path.display());
}

/// The id of the run of migration `file` in `decree_dir` (`.decree/`): the one whose
/// `message.md` has `migration: <file>`. Panics unless there is exactly one.
#[allow(dead_code)]
pub fn migration_run(decree_dir: &Path, file: &str) -> String {
    let line = format!("\nmigration: {file}\n");
    let mut runs: Vec<String> = std::fs::read_dir(decree_dir.join("runs"))
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|id| {
            std::fs::read_to_string(decree_dir.join("runs").join(id).join("message.md"))
                .is_ok_and(|text| text.contains(&line))
        })
        .collect();
    assert_eq!(runs.len(), 1, "runs of {file}: {runs:?}");
    runs.remove(0)
}

/// Whether `id` is a time id, as decree names every run: `YYYYMMDDTHHMMSS.ffffffZ-xxxxxx`.
#[allow(dead_code)]
pub fn is_time_id(id: &str) -> bool {
    let b = id.as_bytes();
    b.len() == 30 && b[8] == b'T' && b[15] == b'.' && b[22] == b'Z' && b[23] == b'-'
}

/// The file `path` of `scripts/<name>/` that the plugin file `plugin` (a path from the
/// repository root, such as `plugins/develop.md`) installs: its text, as installed.
#[allow(dead_code)]
pub fn plugin_script(plugin: &str, path: &str) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(root.join(plugin)).unwrap();
    let yaml = text
        .strip_prefix("---\n")
        .and_then(|t| t.split_once("\n---\n"))
        .map(|(fm, _)| format!("{fm}\n"))
        .unwrap_or_else(|| panic!("{plugin}: no frontmatter"));
    let value: serde_norway::Value = serde_norway::from_str(&yaml).unwrap();
    value["scripts"][path]
        .as_str()
        .unwrap_or_else(|| panic!("{plugin} has no scripts/{path}"))
        .to_string()
}

/// The machine the plugin file `plugin` installs, as `machines/<name>.yml` holds it.
#[allow(dead_code)]
pub fn plugin_machine(plugin: &str) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(root.join(plugin)).unwrap();
    let yaml = text
        .strip_prefix("---\n")
        .and_then(|t| t.split_once("\n---\n"))
        .map(|(fm, _)| format!("{fm}\n"))
        .unwrap_or_else(|| panic!("{plugin}: no frontmatter"));
    let value: serde_norway::Value = serde_norway::from_str(&yaml).unwrap();
    let mut machine = serde_norway::Mapping::new();
    machine.insert("name".into(), value["name"].clone());
    if let serde_norway::Value::Mapping(m) = &value["machine"] {
        for (k, v) in m {
            machine.insert(k.clone(), v.clone());
        }
    }
    serde_norway::to_string(&machine).unwrap()
}
