//! `decree prune` (docs/reference/cli.md; docs/reference/messages.md, Lifecycle step 5): only
//! finished runs older than `--older-than` are deleted, never a run that is not finished, a
//! migration that ended in `failed`, a child of an unfinished parent or a locked run. Each
//! test builds its own `.decree/` in a temp directory and drives the `decree` binary.

use assert_cmd::cargo::cargo_bin_cmd;
use assert_cmd::Command;
use chrono::{SecondsFormat, TimeDelta, Utc};
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::process::Child;
use tempfile::TempDir;

mod common;
use common::write_script;

const MACHINE: &str = "\
name: m
description: One script, then done.
initial: work
states:
  work:
    invoke: work
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
";

/// Logs each run it works for to `order.log`.
const WORK: &str = r#"#!/usr/bin/env bash
echo "$DECREE_MESSAGE_ID" >> "$DECREE_PROJECT_ROOT/order.log"
"#;

/// Long ago: older than `30d`.
const OLD: &str = "2020-01-01T00:00:00.000Z";

struct Project {
    tmp: TempDir,
    /// Holds `locked`'s `.lock` while the project lives.
    holder: Child,
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = self.holder.kill();
        let _ = self.holder.wait();
    }
}

impl Project {
    fn new() -> Project {
        let tmp = TempDir::new().unwrap();
        let decree = tmp.path().join(".decree");
        for dir in ["machines", "scripts", "migrations", "inbox", "runs"] {
            fs::create_dir_all(decree.join(dir)).unwrap();
        }
        fs::write(decree.join("processed.md"), "").unwrap();
        fs::write(decree.join("machines/m.yml"), MACHINE).unwrap();
        write_script(&decree.join("scripts/work"), WORK);
        let holder = std::process::Command::new("sleep")
            .arg("60")
            .spawn()
            .unwrap();
        Project { tmp, holder }
    }

    /// One run of every kind prune tells apart, each `OLD` unless it says otherwise.
    fn with_runs() -> Project {
        let p = Project::new();
        let recent = Utc::now() - TimeDelta::days(1);
        let recent = recent.to_rfc3339_opts(SecondsFormat::Millis, true);
        p.finished("old", "inbox", "", "done", OLD);
        p.finished("old-child", "invoke", "old", "done", OLD);
        p.finished("recent", "inbox", "", "done", &recent);
        // Rule 2: not finished, a failed migration, a child of an unfinished parent.
        p.started("unfinished", "inbox", "", OLD);
        p.finished("01-failed", "migration", "", "failed", OLD);
        p.started("parent", "inbox", "", OLD);
        p.event(
            "parent",
            "inbox",
            OLD,
            json!({"type": "waiting", "child": "child"}),
        );
        p.finished("child", "invoke", "parent", "done", OLD);
        // Rule 3: a live process holds its lock.
        p.finished("locked", "inbox", "", "done", OLD);
        fs::write(p.run_dir("locked").join(".lock"), p.holder.id().to_string()).unwrap();
        p
    }

    fn decree(&self) -> PathBuf {
        self.tmp.path().join(".decree")
    }

    fn run_dir(&self, id: &str) -> PathBuf {
        self.decree().join("runs").join(id)
    }

    /// The run folders left.
    fn runs(&self) -> Vec<String> {
        let mut ids: Vec<String> = fs::read_dir(self.decree().join("runs"))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        ids.sort();
        ids
    }

    /// Append an event with the fields every event carries.
    fn event(&self, id: &str, trigger: &str, ts: &str, fields: Value) {
        let path = self.run_dir(id).join("events.jsonl");
        let mut text = fs::read_to_string(&path).unwrap_or_default();
        let mut event = json!({
            "v": 1, "seq": text.lines().count() + 1, "ts": ts,
            "run_id": id, "machine": "m", "trigger": trigger,
        });
        event
            .as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        text.push_str(&format!("{event}\n"));
        fs::write(path, text).unwrap();
    }

    /// A run claimed into `work` at `ts`, with `parent` unless it is empty.
    fn started(&self, id: &str, trigger: &str, parent: &str, ts: &str) {
        fs::create_dir_all(self.run_dir(id)).unwrap();
        let parent = match parent {
            "" => String::new(),
            p => format!("parent: {p}\n"),
        };
        let message = format!(
            "---\nid: {id}\nmachine: m\n{parent}trigger: {trigger}\nstate: work\n---\nTask.\n"
        );
        fs::write(self.run_dir(id).join("message.md"), message).unwrap();
        let claim = json!({
            "type": "transition", "from": null, "event": "claimed", "to": "work",
            "source": "claim", "exit_code": null,
        });
        self.event(id, trigger, ts, claim);
        fs::write(
            self.run_dir(id).join("0002-work-work.log"),
            "x".repeat(2000),
        )
        .unwrap();
    }

    /// A run that reached final state `state`, with `run_finished` at `ts`.
    fn finished(&self, id: &str, trigger: &str, parent: &str, state: &str, ts: &str) {
        self.started(id, trigger, parent, ts);
        let to = json!({
            "type": "transition", "from": "work", "event": "done", "to": state,
            "source": "exit_code", "exit_code": 0,
        });
        self.event(id, trigger, ts, to);
        let done = json!({"type": "run_finished", "state": state, "duration_ms": 5});
        self.event(id, trigger, ts, done);
    }

    fn cmd(&self, args: &[&str]) -> Command {
        let mut cmd = cargo_bin_cmd!("decree");
        cmd.current_dir(self.tmp.path())
            .env("NO_COLOR", "1")
            .args(args);
        cmd
    }

    /// `decree <args>`: its exit code and stdout.
    fn run(&self, args: &[&str]) -> (i32, String) {
        let out = self.cmd(args).output().unwrap();
        let stdout = String::from_utf8(out.stdout).unwrap();
        (out.status.code().unwrap(), stdout)
    }
}

/// Every run `with_runs` writes.
const ALL: [&str; 8] = [
    "01-failed",
    "child",
    "locked",
    "old",
    "old-child",
    "parent",
    "recent",
    "unfinished",
];

fn all() -> Vec<String> {
    ALL.map(String::from).to_vec()
}

/// What prune prints for `with_runs`, `verb` being `pruned` or `would prune`.
fn listing(verb: &str) -> String {
    format!(
        "{verb} old  m  done  finished {OLD}\n\
         {verb} old-child  m  done  finished {OLD}\n"
    )
}

#[test]
fn prune_deletes_only_finished_runs_older_than_the_age() {
    let p = Project::with_runs();
    let (code, stdout) = p.run(&["prune", "--older-than", "30d"]);
    assert_eq!(code, 0, "{stdout}");
    // Two runs of 2000 bytes of log plus their events and message: 6 KB together.
    assert_eq!(
        stdout,
        format!("{}pruned 2 run(s), 6 KB freed\n", listing("pruned"))
    );
    let kept: Vec<String> = all()
        .into_iter()
        .filter(|id| id != "old" && id != "old-child")
        .collect();
    assert_eq!(p.runs(), kept);
    // A second prune finds nothing more.
    let (code, stdout) = p.run(&["prune", "--older-than", "30d"]);
    assert_eq!(
        (code, stdout.as_str()),
        (0, "pruned 0 run(s), 0 KB freed\n")
    );
}

#[test]
fn dry_run_deletes_nothing_and_lists_the_same_runs() {
    let p = Project::with_runs();
    let (code, stdout) = p.run(&["prune", "--older-than", "30d", "--dry-run"]);
    assert_eq!(code, 0, "{stdout}");
    assert_eq!(
        stdout,
        format!("{}would prune 2 run(s), 6 KB\n", listing("would prune"))
    );
    assert_eq!(p.runs(), all());
    assert!(!p.run_dir("old").join(".lock").exists());
}

#[test]
fn unfinished_runs_failed_migrations_and_children_of_unfinished_parents_are_kept() {
    let p = Project::with_runs();
    // `0m`: every finished run is old enough, so only rule 2 and the lock keep a run.
    let (code, stdout) = p.run(&["prune", "--older-than", "0m"]);
    assert_eq!(code, 0, "{stdout}");
    assert!(stdout.contains("pruned recent  m  done"), "{stdout}");
    assert!(
        stdout.ends_with("pruned 3 run(s), 8 KB freed\n"),
        "{stdout}"
    );
    assert_eq!(
        p.runs(),
        ["01-failed", "child", "locked", "parent", "unfinished"]
    );
}

#[test]
fn a_child_whose_parent_finished_or_is_gone_is_pruned() {
    let p = Project::new();
    p.finished("gone-child", "invoke", "gone", "done", OLD);
    p.finished("done-parent", "inbox", "", "done", OLD);
    p.finished("done-parent-child", "invoke", "done-parent", "done", OLD);
    let (code, _) = p.run(&["prune", "--older-than", "30d"]);
    assert_eq!(code, 0);
    assert!(p.runs().is_empty(), "{:?}", p.runs());
}

#[test]
fn a_run_whose_lock_a_live_process_holds_is_kept() {
    let p = Project::with_runs();
    p.run(&["prune", "--older-than", "0m"]);
    assert!(p.run_dir("locked").is_dir());
    let lock = fs::read_to_string(p.run_dir("locked").join(".lock")).unwrap();
    assert_eq!(lock, p.holder.id().to_string());
}

#[test]
fn a_stale_lock_does_not_keep_a_run() {
    let p = Project::new();
    p.finished("crashed", "inbox", "", "done", OLD);
    // No live process has this pid: the lock is stale.
    fs::write(p.run_dir("crashed").join(".lock"), "999999999").unwrap();
    let (code, stdout) = p.run(&["prune", "--older-than", "30d"]);
    assert_eq!(code, 0, "{stdout}");
    assert!(p.runs().is_empty());
}

#[test]
fn a_bad_or_missing_age_exits_2_and_deletes_nothing() {
    let p = Project::with_runs();
    let cases: [&[&str]; 7] = [
        &["prune"],
        &["prune", "--dry-run"],
        &["prune", "--older-than", "30"],
        &["prune", "--older-than", "d"],
        &["prune", "--older-than", "-1d"],
        &["prune", "--older-than=-1d"],
        &["prune", "--older-than", "1w"],
    ];
    for args in cases {
        let out = p.cmd(args).output().unwrap();
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert!(out.stdout.is_empty(), "{args:?}");
    }
    assert_eq!(p.runs(), all());
}

#[test]
fn a_pruned_migration_does_not_run_again() {
    let p = Project::new();
    fs::write(
        p.decree().join("migrations/01-a.md"),
        "---\nmachine: m\n---\nDo it once.\n",
    )
    .unwrap();
    p.cmd(&["process"]).assert().success();
    assert_eq!(
        fs::read_to_string(p.decree().join("processed.md")).unwrap(),
        "01-a.md\n"
    );
    let (code, stdout) = p.run(&["prune", "--older-than", "0m"]);
    assert_eq!(code, 0, "{stdout}");
    assert!(
        stdout.starts_with("pruned 01-a  m  done  finished "),
        "{stdout}"
    );
    assert!(p.runs().is_empty());

    p.cmd(&["process"]).assert().success();
    assert!(p.runs().is_empty());
    let order = fs::read_to_string(p.tmp.path().join("order.log")).unwrap();
    assert_eq!(order, "01-a\n");
}

/// A message rejected at claim ends with `run_finished`, so prune can delete its run.
#[test]
fn a_rejected_message_is_pruned() {
    let p = Project::new();
    fs::write(
        p.decree().join("inbox/bad.md"),
        "---\nid: bad\nmachine: nope\n---\nTask.\n",
    )
    .unwrap();
    p.cmd(&["process"]).assert().code(1);
    assert_eq!(p.runs(), ["bad"]);
    let (code, stdout) = p.run(&["prune", "--older-than", "0m"]);
    assert_eq!(code, 0, "{stdout}");
    assert!(
        stdout.starts_with("pruned bad  nope  failed  finished "),
        "{stdout}"
    );
    assert!(p.runs().is_empty());
}
