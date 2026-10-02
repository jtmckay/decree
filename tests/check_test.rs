//! `decree check` against the fixtures in `tests/fixtures/check/` (spec section 5,
//! Validation). Each `<case>/pass/` and `<case>/fail/` is copied into a temp project as its
//! `.decree/`; `<case>/expected.txt` is the exact stdout of the failing one.

use assert_cmd::cargo::cargo_bin_cmd;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const CONFIG: &str = "commands:\n  ai_router: \"true {prompt}\"\n  ai_interactive: \"true\"\n";

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Copy `src` to `dst` recursively; `fs::copy` keeps the execute bits.
fn copy_dir(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let to = dst.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &to);
        } else {
            fs::copy(entry.path(), &to).unwrap();
        }
    }
}

/// A temp project whose `.decree/` is a copy of `tree`, with a default `config.yml` unless
/// the tree has its own.
fn project(tree: &Path) -> TempDir {
    let tmp = TempDir::new().unwrap();
    let decree = tmp.path().join(".decree");
    copy_dir(tree, &decree);
    if !decree.join("config.yml").exists() {
        fs::write(decree.join("config.yml"), CONFIG).unwrap();
    }
    tmp
}

/// Run `decree check` in `tmp`: (exit code, stdout).
fn check(tmp: &TempDir) -> (i32, String) {
    let out = cargo_bin_cmd!("decree")
        .current_dir(tmp.path())
        .env("NO_COLOR", "1")
        .arg("check")
        .output()
        .unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stdout).unwrap(),
    )
}

/// The valid tree exits 0 and prints nothing; the invalid one exits 1 and prints exactly
/// `expected.txt`, which names the rule.
fn run_case(case: &str) {
    let dir = fixtures().join("check").join(case);
    let rule = case
        .split('-')
        .next()
        .unwrap()
        .to_uppercase()
        .replace("V0", "V");

    let (code, stdout) = check(&project(&dir.join("pass")));
    assert_eq!((code, stdout.as_str()), (0, ""), "{case}/pass");

    let (code, stdout) = check(&project(&dir.join("fail")));
    let expected = fs::read_to_string(dir.join("expected.txt")).unwrap();
    assert_eq!(code, 1, "{case}/fail: {stdout}");
    assert_eq!(stdout, expected, "{case}/fail");
    for line in stdout.lines() {
        assert!(line.ends_with(&format!("({rule})")), "{case}: {line}");
    }
}

#[test]
fn check_v1_name() {
    run_case("v01-name");
}

#[test]
fn check_v2_state_ids() {
    run_case("v02-state-ids");
}

#[test]
fn check_v3_initial() {
    run_case("v03-initial");
}

#[test]
fn check_v4_targets() {
    run_case("v04-targets");
}

#[test]
fn check_v5_failed() {
    run_case("v05-failed");
}

#[test]
fn check_v6_compound() {
    run_case("v06-compound");
}

#[test]
fn check_v7_final() {
    run_case("v07-final");
}

#[test]
fn check_v8_done() {
    run_case("v08-done");
}

#[test]
fn check_v9_router() {
    run_case("v09-router");
}

#[test]
fn check_v10_cond() {
    run_case("v10-cond");
}

#[test]
fn check_v11_reachable() {
    run_case("v11-reachable");
}

#[test]
fn check_v12_scripts() {
    run_case("v12-scripts");
}

#[test]
fn check_v13_emits() {
    run_case("v13-emits");
}

#[test]
fn check_v14_data() {
    run_case("v14-data");
}

#[test]
fn check_m1_migrations() {
    run_case("m1-migrations");
}

#[test]
fn check_m2_inbox() {
    run_case("m2-inbox");
}

#[test]
fn check_m3_cron() {
    run_case("m3-cron");
}

#[test]
fn check_several_invalid_machines_prints_one_line_per_error() {
    let dir = fixtures().join("check/many");
    let (code, stdout) = check(&project(&dir.join("fail")));
    let expected = fs::read_to_string(dir.join("expected.txt")).unwrap();
    assert_eq!(code, 1);
    assert_eq!(stdout, expected);
    // Four machines, each wrong in its own way: every error is its own line.
    let files: Vec<&str> = stdout
        .lines()
        .map(|l| l.split(':').next().unwrap())
        .collect();
    assert_eq!(files.len(), 6, "{stdout}");
    for id in ["a", "b", "c", "d"] {
        assert!(
            files.contains(&format!("machines/{id}.yml").as_str()),
            "{stdout}"
        );
    }
}

#[test]
fn check_passes_on_the_section_5_examples() {
    let tmp = TempDir::new().unwrap();
    let decree = tmp.path().join(".decree");
    copy_dir(&fixtures().join("machines"), &decree.join("machines"));
    fs::write(decree.join("config.yml"), CONFIG).unwrap();
    let scripts = decree.join("scripts");
    fs::create_dir_all(&scripts).unwrap();
    for name in [
        "greet",
        "build",
        "ask_person",
        "ship",
        "git_baseline",
        "notify",
        "precheck",
        "implement",
        "snapshot",
        "collect_logs",
        "verify",
        "spawn",
        "commit",
    ] {
        let path = scripts.join(name);
        fs::write(&path, "#!/usr/bin/env bash\nexit 0\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    assert_eq!(check(&tmp), (0, String::new()));
}

#[test]
fn check_outside_a_project_fails() {
    let tmp = TempDir::new().unwrap();
    let out = cargo_bin_cmd!("decree")
        .current_dir(tmp.path())
        .arg("check")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
}
