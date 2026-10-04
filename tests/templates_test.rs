//! The examples show projects as `decree init` writes them: every example file that `init`
//! also writes from `src/templates/` is byte-identical to what `decree init --ai claude`
//! writes (the examples' routers ask Claude). A machine two examples need, such as `router`,
//! is a copy in each, and each copy is held to the template. Runs `init` in a temp
//! directory; writes nothing here.

use assert_cmd::cargo::cargo_bin_cmd;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// An example file with a counterpart in `src/templates/`: (path under `.decree/`, template).
/// `init` writes the file at the same path under `.decree/`; the router's files are templates
/// it fills for the backend.
type Pair = (&'static str, &'static str);

const ROUTER: &[Pair] = &[
    ("machines/router.yml", "router/router.yml"),
    ("scripts/router/ask_claude.sh", "router/ask.sh"),
];
const GIT_SCRIPTS: &[Pair] = &[
    ("scripts/git_baseline.sh", "scripts/git_baseline.sh"),
    ("scripts/snapshot.sh", "scripts/snapshot.sh"),
];

/// Each example and the pairs it holds.
const EXAMPLES: &[(&str, &[&[Pair]])] = &[
    ("feature", &[GIT_SCRIPTS, ROUTER]),
    ("sort-documents", &[ROUTER]),
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

/// One line per pair that is missing on either side or whose example file (under
/// `example`, a `.decree/` named `label` in the messages) differs from the file `init`
/// wrote, naming the pair.
fn mismatches(
    label: &str,
    example: &Path,
    pairs: &[Pair],
    templates: &Path,
    written: &Path,
) -> Vec<String> {
    let mut out = Vec::new();
    for (path, template) in pairs {
        let pair = format!("{label}/{path} <-> src/templates/{template}");
        if !templates.join(template).is_file() {
            out.push(format!("{pair}: template missing"));
            continue;
        }
        let Ok(example_bytes) = fs::read(example.join(path)) else {
            out.push(format!("{pair}: example file missing"));
            continue;
        };
        let Ok(init_bytes) = fs::read(written.join(path)) else {
            out.push(format!("{pair}: decree init did not write .decree/{path}"));
            continue;
        };
        if example_bytes != init_bytes {
            out.push(format!("{pair}: differs"));
        }
    }
    out
}

#[test]
fn example_files_match_the_templates_init_writes() {
    let tmp = init_claude();
    let mut found = Vec::new();
    for (name, groups) in EXAMPLES {
        for pairs in *groups {
            found.extend(mismatches(
                &format!("examples/{name}/.decree"),
                &repo().join("examples").join(name).join(".decree"),
                pairs,
                &repo().join("src/templates"),
                &tmp.path().join(".decree"),
            ));
        }
    }
    assert!(found.is_empty(), "{}", found.join("\n"));
}

/// A one-byte change to an example file fails the check, naming that pair.
#[test]
fn one_byte_change_in_an_example_file_names_the_pair() {
    let tmp = init_claude();
    let example = TempDir::new().unwrap();
    for (path, _) in GIT_SCRIPTS {
        let to = example.path().join(path);
        fs::create_dir_all(to.parent().unwrap()).unwrap();
        fs::copy(repo().join("examples/feature/.decree").join(path), &to).unwrap();
    }
    let snapshot = example.path().join("scripts/snapshot.sh");
    let mut bytes = fs::read(&snapshot).unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    fs::write(&snapshot, bytes).unwrap();

    let found = mismatches(
        "examples/feature/.decree",
        example.path(),
        GIT_SCRIPTS,
        &repo().join("src/templates"),
        &tmp.path().join(".decree"),
    );
    assert_eq!(
        found,
        ["examples/feature/.decree/scripts/snapshot.sh <-> src/templates/scripts/snapshot.sh: differs"]
    );
}

/// A listed file missing on either side fails the check.
#[test]
fn missing_example_or_template_file_names_the_pair() {
    let tmp = init_claude();
    let empty = TempDir::new().unwrap();
    let found = mismatches(
        "x",
        empty.path(),
        ROUTER,
        &repo().join("src/templates"),
        &tmp.path().join(".decree"),
    );
    assert_eq!(found.len(), ROUTER.len());
    assert!(found
        .iter()
        .all(|line| line.ends_with("example file missing")));

    let found = mismatches(
        "x",
        &repo().join("examples/sort-documents/.decree"),
        ROUTER,
        empty.path(),
        &tmp.path().join(".decree"),
    );
    assert_eq!(found.len(), ROUTER.len());
    assert!(found.iter().all(|line| line.ends_with("template missing")));
}
