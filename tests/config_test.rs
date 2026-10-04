//! No configuration file (docs/reference/README.md): a `.decree/config.yml` left from 0.4 fails every
//! command that opens the project and names the layout migration script, and a message with no
//! `machine:` is invalid (M1–M3). Each test runs `decree init` in its own temp directory.

use assert_cmd::cargo::cargo_bin_cmd;
use std::fs;
use tempfile::TempDir;

const LEGACY: &str =
    ".decree/config.yml is not used by decree 0.5; run scripts/migrate-0.4-to-0.5.sh";

/// A fresh `decree init` project.
fn init() -> TempDir {
    let dir = TempDir::new().unwrap();
    let out = decree(&dir, &["init", "--ai", "claude"]);
    assert_eq!(out.0, 0, "{}", out.2);
    dir
}

/// Run `decree <args>` in `dir`: (exit code, stdout, stderr).
fn decree(dir: &TempDir, args: &[&str]) -> (i32, String, String) {
    let out = cargo_bin_cmd!("decree")
        .current_dir(dir.path())
        .env("HOME", dir.path())
        .env("NO_COLOR", "1")
        .args(args)
        .output()
        .unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

/// AC: a project with a `.decree/config.yml`; `decree check`, `decree process` and
/// `decree status` (and every other project command) exit 1 with the item 2 message.
#[test]
fn legacy_config_fails_every_project_command() {
    let dir = init();
    fs::write(dir.path().join(".decree/config.yml"), "max_attempts: 3\n").unwrap();
    fs::write(
        dir.path().join(".decree/inbox/task.md"),
        "---\nmachine: develop\n---\n# Task\n",
    )
    .unwrap();
    for args in [
        &["check"][..],
        &["process"],
        &["process", "--dry-run"],
        &["status"],
        &["status", "--cron"],
        &["graph"],
        &["emit", "--machine", "develop"],
        &["tail"],
        &["retry", "x"],
        &["event", "x", "done"],
        &[],
    ] {
        let (code, _, stderr) = decree(&dir, args);
        assert_eq!(code, 1, "decree {args:?}: {stderr}");
        assert_eq!(stderr, format!("error: {LEGACY}\n"), "decree {args:?}");
    }
    // Nothing ran.
    assert_eq!(
        fs::read_dir(dir.path().join(".decree/runs"))
            .unwrap()
            .count(),
        0
    );
}

/// The same project without the file is fine.
#[test]
fn a_fresh_project_has_no_config_and_checks() {
    let dir = init();
    assert!(!dir.path().join(".decree/config.yml").exists());
    let (code, stdout, stderr) = decree(&dir, &["check"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
}

/// AC: an inbox message, a migration and a cron file without `machine:`; `decree check`
/// fails M2, M1 and M3 for them, and `decree process --dry-run` runs nothing.
#[test]
fn messages_without_machine_fail_m1_m2_m3() {
    let dir = init();
    let decree_dir = dir.path().join(".decree");
    fs::write(decree_dir.join("inbox/task.md"), "# No machine key\n").unwrap();
    fs::write(
        decree_dir.join("migrations/01-first.md"),
        "---\nparams: {}\n---\n# No machine key\n",
    )
    .unwrap();
    fs::write(
        decree_dir.join("cron/nightly.md"),
        "---\ncron: \"0 2 * * *\"\n---\n# No machine key\n",
    )
    .unwrap();

    let (code, stdout, stderr) = decree(&dir, &["check"]);
    assert_eq!(code, 1, "{stdout}{stderr}");
    assert_eq!(
        stdout,
        "migrations/01-first.md: line 1: no `machine` key (M1)\n\
         inbox/task.md: line 1: no `machine` key (M2)\n\
         cron/nightly.md: line 1: no `machine` key (M3)\n"
    );

    let (code, stdout, stderr) = decree(&dir, &["process", "--dry-run"]);
    assert_eq!(code, 1, "{stdout}{stderr}");
    assert!(stderr.contains("no `machine` key"), "{stderr}");
    assert!(decree_dir.join("inbox/task.md").is_file());
    assert_eq!(fs::read_dir(decree_dir.join("runs")).unwrap().count(), 0);
}
