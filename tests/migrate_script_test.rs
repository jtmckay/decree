//! `scripts/migrate-0.4-to-0.5.sh` (docs/reference/README.md, No configuration file) against the 0.4.2 project in
//! `tests/fixtures/legacy-0.4/`, copied into a temp directory as its `.decree/`.

use assert_cmd::cargo::cargo_bin_cmd;
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

/// 0.4.2's `.decree/.gitignore`. Written by the test, because a committed copy in the
/// fixture would make git ignore the fixture's own `inbox/`, `outbox/` and `runs/`.
const GITIGNORE_0_4: &str = "inbox/\noutbox/\nruns/\n";

const DEVELOP_YML: &str = "\
name: develop
description: Do the work.
initial: work
states:
  work:
    invoke: work
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
";

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
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

/// Every file under `dir`, by relative path, with its bytes.
fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(base, &path, out);
            } else {
                let rel = path.strip_prefix(base).unwrap().to_path_buf();
                out.insert(rel, fs::read(&path).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

/// A temp project whose `.decree/` is the 0.4.2 fixture.
fn project() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let decree = tmp.path().join(".decree");
    copy_dir(&root().join("tests/fixtures/legacy-0.4"), &decree);
    fs::write(decree.join(".gitignore"), GITIGNORE_0_4).unwrap();
    tmp
}

/// Run the script in `tmp`: (exit code, stdout, stderr).
fn migrate(tmp: &TempDir) -> (i32, String, String) {
    let out = Command::new(root().join("scripts/migrate-0.4-to-0.5.sh"))
        .current_dir(tmp.path())
        .env("HOME", tmp.path())
        .output()
        .unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

/// Run `decree <args>` in `tmp`: (exit code, stdout).
fn decree(tmp: &TempDir, args: &[&str]) -> (i32, String) {
    let out = cargo_bin_cmd!("decree")
        .current_dir(tmp.path())
        .env("HOME", tmp.path())
        .env("NO_COLOR", "1")
        .args(args)
        .output()
        .unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stdout).unwrap(),
    )
}

fn listing(stdout: &str) -> &str {
    let at = stdout
        .find("Machines that pending messages ask for")
        .unwrap_or_else(|| panic!("no listing in:\n{stdout}"));
    &stdout[at..]
}

#[test]
fn test_lists_develop_with_each_file_that_asks_for_it() {
    let tmp = project();
    let (code, stdout, stderr) = migrate(&tmp);
    assert_eq!(code, 1, "stdout:\n{stdout}\nstderr:\n{stderr}");
    // The pending migration, the inbox message and the cron file name `develop`, and so
    // does the outbox message moved into inbox/. The processed migration is not listed.
    assert_eq!(
        listing(&stdout),
        "Machines that pending messages ask for but .decree/machines/ lacks:\n  develop\n    \
         .decree/migrations/02-add-feature.md\n    .decree/inbox/D0002-1200-fix-0.md\n    \
         .decree/inbox/followup.md\n    .decree/cron/hourly.md\n"
    );
}

#[test]
fn test_check_and_status_succeed_once_develop_exists() {
    let tmp = project();
    assert_eq!(migrate(&tmp).0, 1);

    let decree_dir = tmp.path().join(".decree");
    fs::create_dir_all(decree_dir.join("machines")).unwrap();
    fs::create_dir_all(decree_dir.join("scripts")).unwrap();
    fs::write(decree_dir.join("machines/develop.yml"), DEVELOP_YML).unwrap();
    let work = decree_dir.join("scripts/work");
    fs::write(&work, "#!/usr/bin/env bash\necho work\n").unwrap();
    fs::set_permissions(&work, fs::Permissions::from_mode(0o755)).unwrap();

    let (code, stdout) = decree(&tmp, &["check"]);
    assert_eq!(code, 0, "decree check:\n{stdout}");
    let (code, stdout) = decree(&tmp, &["status"]);
    assert_eq!(code, 0, "decree status:\n{stdout}");
    assert!(stdout.contains("D0002-1200-fix-0.md"), "{stdout}");
    assert!(stdout.contains("02-add-feature.md"), "{stdout}");

    // Nothing is missing any more, so a second run lists nothing and exits 0.
    let (code, stdout, _) = migrate(&tmp);
    assert_eq!(code, 0, "{stdout}");
    assert!(!stdout.contains("Machines that"), "{stdout}");
}

#[test]
fn test_lists_rust_develop_as_not_a_valid_machine_name() {
    let tmp = project();
    fs::write(
        tmp.path().join(".decree/migrations/03-rust-change.md"),
        "---\nroutine: rust-develop\n---\nChange the Rust code.\n",
    )
    .unwrap();
    let (code, stdout, _) = migrate(&tmp);
    assert_eq!(code, 1);
    let listing = listing(&stdout);
    assert!(
        listing.contains(
            "  rust-develop: not a valid machine name (^[a-z][a-z0-9_]*$); finish these under \
             decree 0.4 first, since migrations are immutable\n    \
             .decree/migrations/03-rust-change.md\n"
        ),
        "{listing}"
    );
}

#[test]
fn test_keeps_ledger_and_archives_the_0_4_layout() {
    let tmp = project();
    let decree_dir = tmp.path().join(".decree");
    let migrations = snapshot(&decree_dir.join("migrations"));
    let processed = fs::read(decree_dir.join("processed.md")).unwrap();

    migrate(&tmp);

    assert_eq!(snapshot(&decree_dir.join("migrations")), migrations);
    assert_eq!(
        fs::read(decree_dir.join("processed.md")).unwrap(),
        processed
    );

    // 0.4 run folders (no events.jsonl) are archived; 0.5 runs stay.
    let legacy = decree_dir.join("legacy-0.4");
    assert!(legacy.join("runs/D0001-0900-01-setup-0/run.json").is_file());
    assert!(!decree_dir.join("runs/D0001-0900-01-setup-0").exists());
    assert!(decree_dir
        .join("runs/20261001T120000Z-0b12aa/events.jsonl")
        .is_file());

    assert_eq!(
        fs::read_to_string(decree_dir.join(".gitignore")).unwrap(),
        "inbox/\nruns/\n"
    );

    // Pending outbox files join inbox/; the removed paths keep their place under legacy-0.4/.
    assert!(decree_dir.join("inbox/followup.md").is_file());
    assert!(decree_dir.join("inbox/D0002-1200-fix-0.md").is_file());
    for removed in [
        "config.yml",
        "outbox",
        "inbox/dead",
        "router.md",
        "routines",
        "prompts",
    ] {
        assert!(!decree_dir.join(removed).exists(), "{removed} still there");
        assert!(legacy.join(removed).exists(), "{removed} not archived");
    }
    assert!(legacy.join("outbox/dead/rejected.md").is_file());
    assert!(legacy.join("inbox/dead/D0001-0950-broken-0.md").is_file());
    assert!(!legacy.join("outbox/followup.md").exists());

    // config.yml is archived as it was: 0.5 has no configuration file.
    assert_eq!(
        fs::read(legacy.join("config.yml")).unwrap(),
        fs::read(root().join("tests/fixtures/legacy-0.4/config.yml")).unwrap()
    );
}

#[test]
fn test_refuses_to_overwrite_and_changes_nothing() {
    let tmp = project();
    let decree_dir = tmp.path().join(".decree");
    // An inbox file with the outbox file's name: moving it would overwrite.
    fs::write(decree_dir.join("inbox/followup.md"), "queued\n").unwrap();
    let before = snapshot(&decree_dir);

    let (code, _, stderr) = migrate(&tmp);
    assert_eq!(code, 2);
    assert!(
        stderr.contains(".decree/inbox/followup.md exists"),
        "{stderr}"
    );
    assert_eq!(snapshot(&decree_dir), before);
}

#[test]
fn test_without_decree_dir_exits_2() {
    let tmp = TempDir::new().unwrap();
    let (code, _, stderr) = migrate(&tmp);
    assert_eq!(code, 2);
    assert!(stderr.contains("no .decree/"), "{stderr}");
}

/// Run `decree check` in `tmp`: (exit code, stderr).
fn check_stderr(tmp: &TempDir) -> (i32, String) {
    let out = cargo_bin_cmd!("decree")
        .current_dir(tmp.path())
        .env("HOME", tmp.path())
        .env("NO_COLOR", "1")
        .arg("check")
        .output()
        .unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

#[test]
fn test_check_rejects_the_0_4_config_until_migrated() {
    let tmp = project();
    let (code, stderr) = check_stderr(&tmp);
    assert_eq!(code, 1, "{stderr}");
    assert_eq!(
        stderr,
        "error: .decree/config.yml is not used by decree 0.5; run scripts/migrate-0.4-to-0.5.sh\n"
    );
    migrate(&tmp);
    assert!(!tmp.path().join(".decree/config.yml").exists());
    let (_, stderr) = check_stderr(&tmp);
    assert!(!stderr.contains("config.yml"), "{stderr}");
}

/// 0.5 has no default machine: each pending message that names none is listed, and the
/// script exits 1. A reply names the run it answers, not a machine, and is not listed.
#[test]
fn test_lists_each_pending_message_that_names_no_machine() {
    let tmp = project();
    let decree_dir = tmp.path().join(".decree");
    fs::write(decree_dir.join("inbox/bare.md"), "No frontmatter.\n").unwrap();
    fs::write(
        decree_dir.join("migrations/03-bare.md"),
        "---\nparams: {}\n---\nNo machine.\n",
    )
    .unwrap();
    fs::write(
        decree_dir.join("inbox/reply.md"),
        "---\nto: 02-add-feature.w1\nevent: retry\n---\n",
    )
    .unwrap();
    let (code, stdout, stderr) = migrate(&tmp);
    assert_eq!(code, 1, "stdout:\n{stdout}\nstderr:\n{stderr}");
    let at = stdout
        .find("Pending messages that name no machine")
        .unwrap_or_else(|| panic!("no listing in:\n{stdout}"));
    let listed = &stdout[at..stdout.find("Machines that").unwrap()];
    assert!(
        listed.ends_with(
            "immutable):\n    .decree/migrations/03-bare.md\n    .decree/inbox/bare.md\n\n"
        ),
        "{listed}"
    );
    assert!(!listed.contains("reply.md"), "{listed}");

    // With develop present, only the unnamed messages are left, and the script still exits 1.
    fs::create_dir_all(decree_dir.join("machines")).unwrap();
    fs::write(decree_dir.join("machines/develop.yml"), DEVELOP_YML).unwrap();
    let (code, stdout, _) = migrate(&tmp);
    assert_eq!(code, 1, "{stdout}");
    assert!(stdout.contains(".decree/inbox/bare.md"), "{stdout}");
    assert!(!stdout.contains("Machines that"), "{stdout}");
}
