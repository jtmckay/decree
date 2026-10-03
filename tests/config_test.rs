//! A strict `config.yml` (spec section 3): unknown keys fail every command that reads it,
//! 0.4 keys point to the M5.4 script, and without `default_machine` a message with no
//! `machine:` is invalid (M2). Each test runs `decree init` in its own temp directory.

use assert_cmd::cargo::cargo_bin_cmd;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

const HINT: &str = "this is a 0.4 config; run scripts/migrate-0.4-to-0.5.sh";

/// A fresh `decree init` project.
fn init() -> TempDir {
    let dir = TempDir::new().unwrap();
    let out = decree(&dir, &["init"]);
    assert_eq!(out.0, 0, "{}", out.2);
    dir
}

fn config_path(dir: &TempDir) -> PathBuf {
    dir.path().join(".decree/config.yml")
}

fn append_config(dir: &TempDir, text: &str) {
    let mut config = fs::read_to_string(config_path(dir)).unwrap();
    config.push_str(text);
    fs::write(config_path(dir), config).unwrap();
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

/// AC: `bogus: 1` appended to a fresh config; `decree check` exits 1 and names `bogus`.
#[test]
fn unknown_key_fails_check_and_names_it() {
    let dir = init();
    append_config(&dir, "bogus: 1\n");
    let (code, stdout, stderr) = decree(&dir, &["check"]);
    assert_eq!(code, 1, "{stdout}{stderr}");
    assert!(
        stderr.contains("config.yml: unknown field `bogus`"),
        "{stderr}"
    );
    assert!(!stderr.contains(HINT), "{stderr}");
}

/// Every command that reads the config fails the same way.
#[test]
fn unknown_key_fails_every_command_that_reads_the_config() {
    let dir = init();
    append_config(&dir, "bogus: 1\n");
    for args in [
        &["process"][..],
        &["process", "--dry-run"],
        &["graph"],
        &["status"],
        &["status", "--cron"],
        &["emit", "--machine", "develop"],
    ] {
        let (code, _, stderr) = decree(&dir, args);
        assert_eq!(code, 1, "decree {args:?}: {stderr}");
        assert!(
            stderr.contains("config.yml: unknown field `bogus`"),
            "decree {args:?}: {stderr}"
        );
    }
    assert_eq!(
        fs::read_dir(dir.path().join(".decree/runs"))
            .unwrap()
            .count(),
        0
    );
}

/// AC: `routines: {}` or `default_routine: develop`; `decree check` exits 1 and points to
/// the migration script.
#[test]
fn key_from_0_4_fails_check_with_migration_hint() {
    for extra in [
        "routines: {}\n",
        "default_routine: develop\n",
        "max_retries: 3\n",
    ] {
        let dir = init();
        let config = fs::read_to_string(config_path(&dir)).unwrap();
        // Drop `default_machine` so the 0.4 key is the only problem.
        let config: String = config
            .lines()
            .filter(|l| !l.starts_with("default_machine:"))
            .map(|l| format!("{l}\n"))
            .collect();
        fs::write(config_path(&dir), config + extra).unwrap();
        let (code, stdout, stderr) = decree(&dir, &["check"]);
        assert_eq!(code, 1, "{extra}: {stdout}{stderr}");
        let key = extra.split(':').next().unwrap();
        assert!(
            stderr.contains(&format!("config.yml: unknown field `{key}`")),
            "{stderr}"
        );
        assert!(stderr.contains(HINT), "{stderr}");
    }
}

/// AC: no `default_machine` and an inbox message with no `machine:`; `decree check` and
/// `decree process --dry-run` both report it as invalid (M2), and nothing runs.
#[test]
fn message_without_machine_is_invalid_without_default_machine() {
    let dir = init();
    let config = fs::read_to_string(config_path(&dir)).unwrap();
    let config: String = config
        .lines()
        .filter(|l| !l.starts_with("default_machine:"))
        .map(|l| format!("{l}\n"))
        .collect();
    fs::write(config_path(&dir), config).unwrap();
    let inbox = dir.path().join(".decree/inbox");
    fs::write(inbox.join("task.md"), "# No machine key\n").unwrap();
    let expected = "inbox/task.md: line 1: no `machine` key and `default_machine` is not set (M2)";

    let (code, stdout, stderr) = decree(&dir, &["check"]);
    assert_eq!(code, 1, "{stdout}{stderr}");
    assert_eq!(stdout, format!("{expected}\n"));

    let (code, stdout, stderr) = decree(&dir, &["process", "--dry-run"]);
    assert_eq!(code, 1, "{stdout}{stderr}");
    assert!(stdout.contains("task.md"), "{stdout}");
    assert!(stdout.contains("→ invalid"), "{stdout}");
    assert!(stderr.contains(expected), "{stderr}");

    assert!(inbox.join("task.md").is_file());
    assert_eq!(
        fs::read_dir(dir.path().join(".decree/runs"))
            .unwrap()
            .count(),
        0
    );
}

/// With `default_machine` set (as `decree init` writes it), the same message is valid.
#[test]
fn message_without_machine_goes_to_default_machine() {
    let dir = init();
    fs::write(
        dir.path().join(".decree/inbox/task.md"),
        "# No machine key\n",
    )
    .unwrap();
    let (code, stdout, stderr) = decree(&dir, &["process", "--dry-run"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("→ develop"), "{stdout}");
}
