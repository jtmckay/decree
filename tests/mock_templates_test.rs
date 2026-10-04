//! `mock/` shows a project as `decree init` writes it: every mock file that `init` also
//! writes from `src/templates/` is byte-identical to what `decree init --ai claude` writes
//! (the mock's router asks Claude). Runs `init` in a temp directory; writes nothing here.

use assert_cmd::cargo::cargo_bin_cmd;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// Each mock file under `mock/.decree/` with a counterpart in `src/templates/`: (path under
/// `.decree/`, template). `init` writes the file at the same path under `.decree/`; the
/// router's files are templates it fills for the backend.
const PAIRS: &[(&str, &str)] = &[
    ("scripts/git_baseline.sh", "scripts/git_baseline.sh"),
    ("scripts/snapshot.sh", "scripts/snapshot.sh"),
    ("machines/router.yml", "router/router.yml"),
    ("scripts/router/ask_claude.sh", "router/ask.sh"),
];

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// A temp project made by `decree init --ai claude`.
fn init_claude() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let out = cargo_bin_cmd!("decree")
        .current_dir(tmp.path())
        .env("NO_COLOR", "1")
        .args(["init", "--ai", "claude"])
        .output()
        .unwrap();
    assert!(out.status.success(), "decree init failed: {out:?}");
    tmp
}

/// One line per pair that is missing on either side or whose mock file differs from the
/// file `init` wrote, naming the pair.
fn mismatches(mock: &Path, templates: &Path, written: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for (path, template) in PAIRS {
        let pair = format!("mock/.decree/{path} <-> src/templates/{template}");
        if !templates.join(template).is_file() {
            out.push(format!("{pair}: template missing"));
            continue;
        }
        let Ok(mock_bytes) = fs::read(mock.join(path)) else {
            out.push(format!("{pair}: mock file missing"));
            continue;
        };
        let Ok(init_bytes) = fs::read(written.join(path)) else {
            out.push(format!("{pair}: decree init did not write .decree/{path}"));
            continue;
        };
        if mock_bytes != init_bytes {
            out.push(format!("{pair}: differs"));
        }
    }
    out
}

#[test]
fn mock_files_match_the_templates_init_writes() {
    let tmp = init_claude();
    let found = mismatches(
        &repo().join("mock/.decree"),
        &repo().join("src/templates"),
        &tmp.path().join(".decree"),
    );
    assert!(found.is_empty(), "{}", found.join("\n"));
}

/// A one-byte change to a mock file fails the check, naming that pair.
#[test]
fn one_byte_change_in_a_mock_file_names_the_pair() {
    let tmp = init_claude();
    let mock = TempDir::new().unwrap();
    for (path, _) in PAIRS {
        let to = mock.path().join(path);
        fs::create_dir_all(to.parent().unwrap()).unwrap();
        fs::copy(repo().join("mock/.decree").join(path), &to).unwrap();
    }
    let snapshot = mock.path().join("scripts/snapshot.sh");
    let mut bytes = fs::read(&snapshot).unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    fs::write(&snapshot, bytes).unwrap();

    let found = mismatches(
        mock.path(),
        &repo().join("src/templates"),
        &tmp.path().join(".decree"),
    );
    assert_eq!(
        found,
        ["mock/.decree/scripts/snapshot.sh <-> src/templates/scripts/snapshot.sh: differs"]
    );
}

/// A listed file missing on either side fails the check.
#[test]
fn missing_mock_or_template_file_names_the_pair() {
    let tmp = init_claude();
    let empty = TempDir::new().unwrap();
    let found = mismatches(
        empty.path(),
        &repo().join("src/templates"),
        &tmp.path().join(".decree"),
    );
    assert_eq!(found.len(), PAIRS.len());
    assert!(found.iter().all(|line| line.ends_with("mock file missing")));

    let found = mismatches(
        &repo().join("mock/.decree"),
        empty.path(),
        &tmp.path().join(".decree"),
    );
    assert_eq!(found.len(), PAIRS.len());
    assert!(found.iter().all(|line| line.ends_with("template missing")));
}
