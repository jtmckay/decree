//! A cron file whose `cron:` is `@reboot` (docs/reference/messages.md, Cron files): queued
//! once each time `decree daemon` starts, as Vixie cron's `@reboot` fires when the cron
//! daemon starts. Never by `process`, never for a file added while the daemon runs, not
//! for a machine the daemon excludes; queued even while the queue is held. `check` accepts
//! it and `status --cron` says it fires at daemon start.

use assert_cmd::cargo::cargo_bin_cmd;
use assert_cmd::Command;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::{Child, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

mod common;
#[path = "common/schema.rs"]
mod schema;

const BOOT: &str = "\
name: boot
description: Work to do whenever the daemon starts.
initial: work
states:
  work:
    invoke: warm
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
";

const REBOOT: &str = "---\ncron: \"@reboot\"\nmachine: boot\n---\nWarm up.\n";

struct Project {
    tmp: TempDir,
}

impl Project {
    /// `decree init --ai claude`, plus the `boot` machine and its script.
    fn new() -> Project {
        let p = Project {
            tmp: TempDir::new().unwrap(),
        };
        p.decree(&["init", "--ai", "claude"]).assert().success();
        fs::create_dir_all(p.decree_dir().join("machines")).unwrap();
        fs::write(p.decree_dir().join("machines/boot.yml"), BOOT).unwrap();
        common::write_script(
            &p.decree_dir().join("scripts/warm"),
            "#!/usr/bin/env bash\necho warm\n",
        );
        p
    }

    fn root(&self) -> PathBuf {
        self.tmp.path().to_path_buf()
    }

    fn decree_dir(&self) -> PathBuf {
        self.root().join(".decree")
    }

    fn decree(&self, args: &[&str]) -> Command {
        let mut cmd = cargo_bin_cmd!("decree");
        cmd.current_dir(self.root())
            .env("NO_COLOR", "1")
            .env_remove("DECREE_MACHINES")
            .env_remove("DECREE_EXCLUDE_MACHINES")
            .args(args);
        cmd
    }

    fn reboot_file(&self) {
        fs::create_dir_all(self.decree_dir().join("cron")).unwrap();
        fs::write(self.decree_dir().join("cron/boot.md"), REBOOT).unwrap();
    }

    /// `decree daemon --interval 1s <args>` in the background.
    fn daemon(&self, args: &[&str]) -> Child {
        std::process::Command::new(env!("CARGO_BIN_EXE_decree"))
            .current_dir(self.root())
            .env("NO_COLOR", "1")
            .env_remove("DECREE_MACHINES")
            .env_remove("DECREE_EXCLUDE_MACHINES")
            .args(["daemon", "--interval", "1s"])
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap()
    }

    /// Messages for `boot` with `trigger: cron`: queued in `inbox/` or started as runs.
    fn boot_messages(&self) -> Vec<String> {
        let mut texts = Vec::new();
        let inbox = self.decree_dir().join("inbox");
        for e in fs::read_dir(&inbox).into_iter().flatten() {
            texts.push(fs::read_to_string(e.unwrap().path()).unwrap_or_default());
        }
        for e in fs::read_dir(self.decree_dir().join("runs"))
            .into_iter()
            .flatten()
        {
            let path = e.unwrap().path().join("message.md");
            texts.push(fs::read_to_string(path).unwrap_or_default());
        }
        texts
            .into_iter()
            .filter(|t| t.contains("\nmachine: boot\n") && t.contains("\ntrigger: cron\n"))
            .collect()
    }

    /// The `boot` runs that finished `done`.
    fn boot_done(&self) -> usize {
        fs::read_dir(self.decree_dir().join("runs"))
            .into_iter()
            .flatten()
            .filter(|e| {
                let dir = e.as_ref().unwrap().path();
                let events = fs::read_to_string(dir.join("events.jsonl")).unwrap_or_default();
                let message = fs::read_to_string(dir.join("message.md")).unwrap_or_default();
                message.contains("\nmachine: boot\n")
                    && events.lines().last().is_some_and(|l| {
                        let e: Value = serde_json::from_str(l).unwrap();
                        e["type"] == "run_finished"
                    })
            })
            .count()
    }

    fn inbox_len(&self) -> usize {
        fs::read_dir(self.decree_dir().join("inbox"))
            .map(|d| d.count())
            .unwrap_or(0)
    }
}

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting: {what}");
        thread::sleep(Duration::from_millis(50));
    }
}

/// SIGTERM, then wait for exit 0.
fn stop(mut daemon: Child) {
    // SAFETY: sends SIGTERM to the daemon this test spawned.
    unsafe { libc::kill(daemon.id() as i32, libc::SIGTERM) };
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(status) = daemon.try_wait().unwrap() {
            assert_eq!(status.code(), Some(0));
            return;
        }
        assert!(Instant::now() < deadline, "daemon did not exit");
        thread::sleep(Duration::from_millis(20));
    }
}

/// AC: one message per daemon start, however many passes; a second start queues a second.
#[test]
fn reboot_fires_once_per_daemon_start() {
    let p = Project::new();
    p.reboot_file();

    let daemon = p.daemon(&[]);
    wait_for("the first boot run", || p.boot_done() == 1);
    // Several more passes at 1s each.
    thread::sleep(Duration::from_millis(3500));
    stop(daemon);
    let messages = p.boot_messages();
    assert_eq!(messages.len(), 1, "{messages:?}");
    assert!(!messages[0].contains("cron:"), "{}", messages[0]);
    assert!(messages[0].ends_with("---\nWarm up.\n"), "{}", messages[0]);

    let daemon = p.daemon(&[]);
    wait_for("the second boot run", || p.boot_done() == 2);
    thread::sleep(Duration::from_millis(1500));
    stop(daemon);
    assert_eq!(p.boot_messages().len(), 2);
}

/// AC: `decree process` fires no cron file, `@reboot` included.
#[test]
fn process_never_fires_reboot() {
    let p = Project::new();
    p.reboot_file();
    p.decree(&["process"]).assert().success();
    p.decree(&["process"]).assert().success();
    assert!(p.boot_messages().is_empty());
}

/// AC: a `@reboot` file added while the daemon runs waits for the next start.
#[test]
fn reboot_file_added_while_running_waits_for_next_start() {
    let p = Project::new();
    let daemon = p.daemon(&[]);
    // Let the first pass go by, then add the file and let more passes go by.
    thread::sleep(Duration::from_millis(1500));
    p.reboot_file();
    thread::sleep(Duration::from_millis(3000));
    stop(daemon);
    assert!(p.boot_messages().is_empty());

    let daemon = p.daemon(&[]);
    wait_for("the boot run after restart", || p.boot_done() == 1);
    stop(daemon);
    assert_eq!(p.boot_messages().len(), 1);
}

/// AC: `--exclude-machine boot` (and `DECREE_EXCLUDE_MACHINES`) leave it alone; `--machine
/// boot` fires it.
#[test]
fn excluded_machine_does_not_fire_reboot() {
    let p = Project::new();
    p.reboot_file();
    let daemon = p.daemon(&["--exclude-machine", "boot"]);
    thread::sleep(Duration::from_millis(2500));
    stop(daemon);
    assert!(p.boot_messages().is_empty());

    let daemon = std::process::Command::new(env!("CARGO_BIN_EXE_decree"))
        .current_dir(p.root())
        .env("DECREE_EXCLUDE_MACHINES", "boot")
        .env_remove("DECREE_MACHINES")
        .args(["daemon", "--interval", "1s"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    thread::sleep(Duration::from_millis(2500));
    stop(daemon);
    assert!(p.boot_messages().is_empty());

    let daemon = p.daemon(&["--machine", "boot"]);
    wait_for("the boot run", || p.boot_done() == 1);
    stop(daemon);
    assert_eq!(p.boot_messages().len(), 1);
}

/// AC: with a holding attention file the message is queued at start and runs only once
/// the file is deleted.
#[test]
fn reboot_is_queued_while_held_and_runs_after() {
    let p = Project::new();
    p.reboot_file();
    p.decree(&["stop"]).assert().success();
    let daemon = p.daemon(&[]);
    wait_for("the queued boot message", || p.boot_messages().len() == 1);
    thread::sleep(Duration::from_millis(2500));
    assert_eq!(p.inbox_len(), 1, "held: still in inbox");
    assert_eq!(p.boot_done(), 0, "held: not run");

    fs::remove_file(p.decree_dir().join("attention/manual-stop.md")).unwrap();
    wait_for("the boot run after the hold", || p.boot_done() == 1);
    stop(daemon);
    assert_eq!(p.boot_messages().len(), 1);
}

/// AC: `check` accepts `@reboot`; `status --cron --format json` validates against the
/// schema and says it fires at daemon start; the text says `daemon start`.
#[test]
fn check_and_status_cron_know_reboot() {
    let p = Project::new();
    p.reboot_file();
    fs::write(
        p.decree_dir().join("cron/hourly.md"),
        "---\ncron: \"0 * * * *\"\nmachine: boot\n---\nHourly.\n",
    )
    .unwrap();
    p.decree(&["check"]).assert().success();

    let out = p
        .decree(&["status", "--cron", "--format", "json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).unwrap();
    let validator = schema::validator(schema::CLI_STATUS_SCHEMA);
    assert_eq!(
        schema::json_errors(&validator, &text),
        Some(vec![]),
        "{text}"
    );
    let doc: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        doc["cron"][0],
        serde_json::json!({
            "file": "boot.md",
            "cron": "@reboot",
            "machine": "boot",
            "next": null,
            "at_daemon_start": true,
        })
    );
    assert_eq!(doc["cron"][1]["file"], "hourly.md");
    assert_eq!(doc["cron"][1]["at_daemon_start"], false);
    assert!(doc["cron"][1]["next"].is_string(), "{doc:#}");

    let out = p
        .decree(&["status", "--cron"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).unwrap();
    let line = text.lines().find(|l| l.starts_with("boot.md")).unwrap();
    assert!(
        line.contains("@reboot") && line.ends_with("daemon start"),
        "{text}"
    );
}

/// `check` rejects other `@` words: `@reboot` is the only new spelling.
#[test]
fn check_rejects_daemon_alias() {
    let p = Project::new();
    fs::create_dir_all(p.decree_dir().join("cron")).unwrap();
    fs::write(
        p.decree_dir().join("cron/boot.md"),
        "---\ncron: \"@daemon\"\nmachine: boot\n---\n",
    )
    .unwrap();
    p.decree(&["check"]).assert().failure();
}
