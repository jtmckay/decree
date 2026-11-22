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

/// The id of the run of `file` in `decree_dir` (`.decree/`): an inbox message's, whose claim
/// event names the file it arrived as, or an order file's, whose `message.md` has `order:
/// <machine>/<file>`. Panics unless there is exactly one.
#[allow(dead_code)]
pub fn run_of(decree_dir: &Path, file: &str) -> String {
    let runs = decree_dir.join("runs");
    let mut ids: Vec<String> = std::fs::read_dir(&runs)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|id| {
            let dir = runs.join(id);
            let claim = std::fs::read_to_string(dir.join("events.jsonl"))
                .ok()
                .and_then(|t| t.lines().next().map(String::from))
                .and_then(|l| serde_json::from_str::<serde_json::Value>(&l).ok());
            let arrived = claim.is_some_and(|c| c["file"] == file);
            let order = std::fs::read_to_string(dir.join("message.md")).is_ok_and(|m| {
                m.lines()
                    .skip(1)
                    .take_while(|l| *l != "---")
                    .filter_map(|l| l.strip_prefix("order: "))
                    .any(|o| o.ends_with(&format!("/{file}")))
            });
            arrived || order
        })
        .collect();
    ids.sort();
    assert_eq!(ids.len(), 1, "runs of {file}: {ids:?}");
    ids.remove(0)
}

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

/// Copy each file a test wrote to `<root>/.decree/scripts/` into every machine's own
/// `machines/<m>/scripts/`, where decree resolves scripts (docs/reference/scripts.md,
/// Resolution). Tests write a script once for every machine to run, as a project converted
/// from 0.5's shared `scripts/` would copy it; a machine's own script of that name wins.
/// The copies made are listed in `.decree/scripts/.shared`, so a later call refreshes them.
#[allow(dead_code)]
pub fn share_scripts(root: &Path) {
    let decree = root.join(".decree");
    let flat = decree.join("scripts");
    let machines = decree.join("machines");
    if !flat.is_dir() || !machines.is_dir() {
        return;
    }
    let ledger = flat.join(".shared");
    let mut shared: Vec<String> = std::fs::read_to_string(&ledger)
        .map(|t| t.lines().map(String::from).collect())
        .unwrap_or_default();
    let files: Vec<_> = std::fs::read_dir(&flat)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.path().is_file() && !e.file_name().to_string_lossy().starts_with('.'))
        .collect();
    let installed = |m: &std::fs::DirEntry| {
        decree
            .join("cache/plugins")
            .join(format!("{}.md", m.file_name().to_string_lossy()))
            .is_file()
    };
    for m in std::fs::read_dir(&machines).unwrap().filter_map(Result::ok) {
        // A plugin's installed copy is what the plugin says, nothing added.
        if !m.path().join("machine.yml").is_file() || installed(&m) {
            continue;
        }
        for f in &files {
            let dest = m.path().join("scripts").join(f.file_name());
            let key = dest.to_string_lossy().into_owned();
            if dest.exists() && !shared.contains(&key) {
                continue;
            }
            let text = std::fs::read(f.path()).unwrap();
            if std::fs::read(&dest).ok().as_deref() != Some(text.as_slice()) {
                let mode = std::fs::metadata(f.path()).unwrap().permissions();
                std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
                let tmp = dest.with_extension("sharing");
                std::fs::write(&tmp, &text).unwrap();
                std::fs::set_permissions(&tmp, mode).unwrap();
                std::fs::rename(&tmp, &dest).unwrap();
            }
            if !shared.contains(&key) {
                shared.push(key);
            }
        }
    }
    // A folder `scripts/<m>/` is machine m's own folder, as in 0.5: its files go to
    // `machines/<m>/scripts/`.
    for dir in std::fs::read_dir(&flat).unwrap().filter_map(Result::ok) {
        let m = machines.join(dir.file_name());
        if !dir.path().is_dir() || !m.is_dir() {
            continue;
        }
        copy_tree(&dir.path(), &m.join("scripts"));
    }
    std::fs::write(&ledger, shared.join("\n")).unwrap();
}

/// Copy every file under `from` into `to`, keeping modes, replacing what differs.
fn copy_tree(from: &Path, to: &Path) {
    for entry in std::fs::read_dir(from).unwrap().filter_map(Result::ok) {
        let dest = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &dest);
            continue;
        }
        let text = std::fs::read(entry.path()).unwrap();
        if std::fs::read(&dest).ok().as_deref() == Some(text.as_slice()) {
            continue;
        }
        std::fs::create_dir_all(to).unwrap();
        let tmp = dest.with_extension("sharing");
        std::fs::write(&tmp, &text).unwrap();
        std::fs::set_permissions(&tmp, std::fs::metadata(entry.path()).unwrap().permissions())
            .unwrap();
        std::fs::rename(&tmp, &dest).unwrap();
    }
}

/// `decree` run in `root`, after `share_scripts(root)`.
#[allow(dead_code)]
pub fn decree_at(root: impl AsRef<Path>) -> assert_cmd::Command {
    share_scripts(root.as_ref());
    let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("decree");
    cmd.current_dir(root.as_ref());
    cmd
}

/// Write `text` to `path`, creating its folder first.
#[allow(dead_code)]
pub fn write_file(path: &Path, text: impl AsRef<[u8]>) {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(path, text).unwrap();
}

/// `decree` run in `root` as a `std::process::Command` (for a child the test signals or
/// reads as it runs), after `share_scripts(root)`.
#[allow(dead_code)]
pub fn std_decree_at(root: impl AsRef<Path>) -> std::process::Command {
    share_scripts(root.as_ref());
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_decree"));
    cmd.current_dir(root.as_ref());
    cmd
}

#[allow(dead_code)]
/// Drop the official plugins `decree init` adds (`ai`, `order`, `prune`, `skill`) from the
/// project at `root`
/// the documented way: delete their files, `decree install`, `decree graph`. For a test
/// whose runs and machines are its own.
pub fn drop_default_plugins(root: &Path) {
    for name in ["ai", "order", "prune", "skill"] {
        let file = root.join(format!(".decree/plugins/{name}.md"));
        if file.exists() {
            std::fs::remove_file(&file).unwrap();
        }
    }
    for args in [&["install"][..], &["graph"][..]] {
        let out = std_decree_at(root).args(args).output().unwrap();
        assert!(out.status.success(), "{args:?}: {out:?}");
    }
}
