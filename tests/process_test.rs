//! `decree process` (docs/reference/messages.md, docs/reference/cli.md): inbox claim and validation, and the six
//! migration rules. Each test builds its own `.decree/` in a temp directory.

use assert_cmd::cargo::cargo_bin_cmd;
use assert_cmd::Command;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

mod common;
use common::write_script;

const MACHINE: &str = "\
name: flow
description: One script, then a commit on done.
data:
  fail: { type: bool, default: false }
  followup: { type: bool, default: false }
  commit_fails: { type: bool, default: false }
initial: work
states:
  work:
    invoke: work
    transitions: { done: done }
  done: { final: true, onentry: [commit] }
  failed: { final: true }
";

/// Logs start and end to `order.log`; fails, or emits a follow-up, when the params say so.
const WORK: &str = r#"#!/usr/bin/env bash
echo "start $DECREE_MESSAGE_ID" >> "$DECREE_PROJECT_ROOT/order.log"
if [ "$DECREE_DATA_FAIL" = true ]; then exit 1; fi
if [ "$DECREE_DATA_FOLLOWUP" = true ]; then
  inbox="$DECREE_PROJECT_ROOT/.decree/inbox"
  printf -- '---\nmachine: flow\n---\nfollow-up\n' > "$inbox/.followup.md.tmp"
  mv "$inbox/.followup.md.tmp" "$inbox/followup.md"
fi
echo "end $DECREE_MESSAGE_ID" >> "$DECREE_PROJECT_ROOT/order.log"
"#;

/// Copies the ledger as the commit sees it; fails when the params say so.
const COMMIT: &str = r#"#!/usr/bin/env bash
cp "$DECREE_PROJECT_ROOT/.decree/processed.md" "$DECREE_RUN_DIR/ledger-at-commit.txt"
if [ "$DECREE_DATA_COMMIT_FAILS" = true ]; then exit 1; fi
"#;

struct Project {
    tmp: TempDir,
}

impl Project {
    fn new() -> Project {
        let tmp = TempDir::new().unwrap();
        let decree = tmp.path().join(".decree");
        for dir in ["machines", "scripts", "migrations", "inbox", "runs"] {
            fs::create_dir_all(decree.join(dir)).unwrap();
        }
        fs::write(decree.join("processed.md"), "").unwrap();
        fs::write(decree.join("machines/flow.yml"), MACHINE).unwrap();
        for (name, text) in [("work", WORK), ("commit", COMMIT)] {
            write_script(&decree.join("scripts").join(name), text);
        }
        Project { tmp }
    }

    fn decree(&self) -> PathBuf {
        self.tmp.path().join(".decree")
    }

    fn write(&self, rel: &str, text: &str) {
        fs::write(self.decree().join(rel), text).unwrap();
    }

    fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.decree().join(rel)).unwrap_or_default()
    }

    fn order(&self) -> Vec<String> {
        fs::read_to_string(self.tmp.path().join("order.log"))
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect()
    }

    fn runs(&self) -> Vec<String> {
        let mut runs: Vec<String> = fs::read_dir(self.decree().join("runs"))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        runs.sort();
        runs
    }

    fn events(&self, id: &str) -> Vec<Value> {
        events(&self.decree().join("runs").join(id))
    }

    fn process(&self) -> Command {
        let mut cmd = cargo_bin_cmd!("decree");
        cmd.current_dir(self.tmp.path())
            .env("NO_COLOR", "1")
            .arg("process");
        cmd
    }
}

fn events(run_dir: &Path) -> Vec<Value> {
    fs::read_to_string(run_dir.join("events.jsonl"))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn last_state(events: &[Value]) -> &str {
    events
        .iter()
        .rev()
        .find(|e| e["type"] == "transition")
        .and_then(|e| e["to"].as_str())
        .unwrap()
}

fn stderr(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

// ---------------------------------------------------------------
// Inbox: claim and validation (docs/reference/messages.md, Lifecycle)
// ---------------------------------------------------------------

#[test]
fn unknown_machine_ends_failed_with_invalid_message() {
    let p = Project::new();
    p.write("inbox/a.md", "---\nid: run-a\nmachine: nope\n---\nbody\n");
    let out = p.process().assert().code(1).get_output().clone();
    assert!(
        stderr(&out).contains("unknown machine `nope`"),
        "{}",
        stderr(&out)
    );

    assert_eq!(p.runs(), ["run-a"]);
    assert!(!p.decree().join("inbox/a.md").exists());
    let events = p.events("run-a");
    assert_eq!(events.len(), 1, "{events:?}");
    let e = &events[0];
    assert_eq!(e["type"], "transition");
    assert_eq!(e["to"], "failed");
    assert_eq!(e["source"], "invalid_message");
    assert_eq!(e["file"], "a.md");
    assert!(e["error"]
        .as_str()
        .unwrap()
        .contains("unknown machine `nope`"));
    assert!(p.read("runs/run-a/message.md").contains("state: failed"));
    assert!(p.order().is_empty());
}

#[test]
fn unparsable_message_ends_failed_and_is_left_unchanged() {
    let p = Project::new();
    let text = "---\nmachine: flow\nbody without a closing fence\n";
    p.write("inbox/a.md", text);
    p.process().assert().code(1);
    let run = &p.runs()[0];
    let events = p.events(run);
    assert_eq!(events[0]["source"], "invalid_message");
    assert!(events[0]["error"]
        .as_str()
        .unwrap()
        .contains("message.md: line 1:"));
    assert_eq!(p.read(&format!("runs/{run}/message.md")), text);
}

#[test]
fn routine_key_names_the_machine() {
    let p = Project::new();
    p.write(
        "inbox/a.md",
        "---\nroutine: flow\nkeep: me\n---\r\nbody\r\n",
    );
    p.process().assert().success();
    let run = &p.runs()[0];
    let events = p.events(run);
    assert_eq!(events[0]["machine"], "flow");
    assert_eq!(last_state(&events), "done");
    // decree added `id` and `trigger`, mirrored `state`, and kept the rest.
    let message = p.read(&format!("runs/{run}/message.md"));
    assert_eq!(
        message,
        format!(
            "---\nroutine: flow\nkeep: me\nid: {run}\ntrigger: inbox\nstate: done\n---\nbody\r\n"
        )
    );
}

#[test]
fn inbox_runs_in_filename_order() {
    let p = Project::new();
    p.write("inbox/b.md", "---\nid: run-b\nmachine: flow\n---\n");
    p.write("inbox/a.md", "---\nid: run-a\nmachine: flow\n---\n");
    p.write("inbox/.hidden.md", "---\nmachine: flow\n---\n");
    p.process().assert().success();
    assert_eq!(
        p.order(),
        ["start run-a", "end run-a", "start run-b", "end run-b"]
    );
    assert!(p.decree().join("inbox/.hidden.md").exists());
}

#[test]
fn a_failed_inbox_run_stops_process() {
    let p = Project::new();
    p.write(
        "inbox/a.md",
        "---\nid: run-a\nmachine: flow\nparams: { fail: true }\n---\n",
    );
    p.write("inbox/b.md", "---\nid: run-b\nmachine: flow\n---\n");
    let out = p.process().assert().code(1).get_output().clone();
    assert!(stderr(&out).contains("run run-a (a.md) ended in `failed`"));
    assert_eq!(p.order(), ["start run-a"]);
    assert!(p.decree().join("inbox/b.md").exists());
}

// ---------------------------------------------------------------
// Migrations (docs/reference/messages.md, Migrations, rules 1–6)
// ---------------------------------------------------------------

#[test]
fn rule1_migration_runs_from_a_copy() {
    let p = Project::new();
    let text = "---\nmachine: flow\nid: ignored\ntrigger: inbox\n---\n# Task\n";
    p.write("migrations/01-a.md", text);
    p.process().assert().success();
    assert_eq!(p.read("migrations/01-a.md"), text);
    assert_eq!(p.runs(), ["01-a"]);
    assert_eq!(
        p.read("runs/01-a/message.md"),
        "---\nmachine: flow\nid: 01-a\ntrigger: migration\nstate: done\n---\n# Task\n"
    );
    let events = p.events("01-a");
    assert_eq!(events[0]["trigger"], "migration");
    assert_eq!(events[0]["file"], "01-a.md");
}

#[test]
fn rule2_migration_in_processed_md_is_skipped() {
    let p = Project::new();
    p.write("migrations/01-a.md", "---\nmachine: flow\n---\n");
    p.write("migrations/02-b.md", "---\nmachine: flow\n---\n");
    p.write("processed.md", "01-a.md\n01-a.md\n");
    p.process().assert().success();
    assert_eq!(p.runs(), ["02-b"]);
    assert_eq!(p.order(), ["start 02-b", "end 02-b"]);
    assert_eq!(p.read("processed.md"), "01-a.md\n01-a.md\n02-b.md\n");
}

#[test]
fn rule3_follow_up_finishes_before_the_next_migration() {
    let p = Project::new();
    p.write(
        "migrations/01-a.md",
        "---\nmachine: flow\nparams: { followup: true }\n---\n",
    );
    p.write("migrations/02-b.md", "---\nmachine: flow\n---\n");
    p.process().assert().success();
    let order = p.order();
    assert_eq!(order.len(), 6, "{order:?}");
    assert_eq!(order[..2], ["start 01-a", "end 01-a"]);
    let followup = order[2].strip_prefix("start ").unwrap();
    assert_ne!(followup, "02-b");
    assert_eq!(order[3], format!("end {followup}"));
    assert_eq!(order[4..], ["start 02-b", "end 02-b"]);
    assert_eq!(p.read("processed.md"), "01-a.md\n02-b.md\n");
}

#[test]
fn rule4_failed_migration_blocks_the_next_and_exits_1() {
    let p = Project::new();
    p.write(
        "migrations/01-a.md",
        "---\nmachine: flow\nparams: { fail: true }\n---\n",
    );
    p.write("migrations/02-b.md", "---\nmachine: flow\n---\n");
    let out = p.process().assert().code(1).get_output().clone();
    let err = stderr(&out);
    assert!(err.contains("migration 01-a.md ended in `failed`"), "{err}");
    assert!(err.contains("decree retry 01-a"), "{err}");
    assert_eq!(p.runs(), ["01-a"]);
    assert_eq!(p.order(), ["start 01-a"]);
    assert_eq!(p.read("processed.md"), "");

    // The failed run still blocks the next pass; nothing re-runs.
    let out = p.process().assert().code(1).get_output().clone();
    assert!(stderr(&out).contains("decree retry 01-a"));
    assert_eq!(p.runs(), ["01-a"]);
    assert_eq!(p.order(), ["start 01-a"]);
}

#[test]
fn rule5_ledger_is_written_before_the_final_onentry() {
    let p = Project::new();
    p.write("processed.md", "00-old.md\n");
    p.write("migrations/01-a.md", "---\nmachine: flow\n---\n");
    p.process().assert().success();
    assert_eq!(
        p.read("runs/01-a/ledger-at-commit.txt"),
        "00-old.md\n01-a.md\n"
    );
    assert_eq!(p.read("processed.md"), "00-old.md\n01-a.md\n");
}

#[test]
fn rule5_failing_final_onentry_removes_the_ledger_line() {
    let p = Project::new();
    p.write("processed.md", "00-old.md\n");
    p.write(
        "migrations/01-a.md",
        "---\nmachine: flow\nparams: { commit_fails: true }\n---\n",
    );
    p.write("migrations/02-b.md", "---\nmachine: flow\n---\n");
    p.process().assert().code(1);
    // The commit saw the line; it is gone again, and the run ended in `failed`.
    assert_eq!(
        p.read("runs/01-a/ledger-at-commit.txt"),
        "00-old.md\n01-a.md\n"
    );
    assert_eq!(p.read("processed.md"), "00-old.md\n");
    assert_eq!(last_state(&p.events("01-a")), "failed");
    assert_eq!(p.runs(), ["01-a"]);
}

#[test]
fn rule6_invalid_pending_migration_runs_nothing() {
    let p = Project::new();
    p.write("migrations/01-a.md", "---\nmachine: flow\n---\n");
    p.write("migrations/02-b.md", "---\nmachine: nope\n---\n");
    p.write(
        "migrations/03-c.md",
        "---\nmachine: flow\nparams: { rounds: 2 }\n---\n",
    );
    p.write("migrations/04-d.md", "---\nmachine: flow\n");
    p.write("inbox/a.md", "---\nmachine: flow\n---\n");
    let out = p.process().assert().code(1).get_output().clone();
    let err = stderr(&out);
    assert!(
        err.contains("migrations/02-b.md: line 2: unknown machine `nope`"),
        "{err}"
    );
    assert!(
        err.contains("migrations/03-c.md: line 3: unknown param `rounds`"),
        "{err}"
    );
    assert!(err.contains("migrations/04-d.md: line 1: "), "{err}");
    assert!(
        err.contains("3 migration(s) are invalid; nothing was processed."),
        "{err}"
    );
    assert!(p.runs().is_empty());
    assert!(p.order().is_empty());
    assert!(p.decree().join("inbox/a.md").exists());
    assert_eq!(p.read("processed.md"), "");
}

#[test]
fn dry_run_lists_and_runs_nothing() {
    let p = Project::new();
    p.write("migrations/01-a.md", "---\nmachine: flow\n---\n");
    p.write("inbox/a.md", "---\nroutine: flow\n---\n");
    let out = p
        .process()
        .arg("--dry-run")
        .assert()
        .success()
        .get_output()
        .clone();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("01-a.md"), "{stdout}");
    assert!(stdout.contains("→ flow"), "{stdout}");
    assert!(stdout.contains("a.md"), "{stdout}");
    assert!(p.runs().is_empty());
    assert!(p.order().is_empty());
}
