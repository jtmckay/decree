//! `decree cancel` (docs/reference/cli.md, Cancel): a waiting, pending or interrupted run,
//! and every unfinished child run below it, ends in `failed` with a `cancel` event and
//! `run_finished` with `cancelled: true`. No script runs, no attention file stays, its queued
//! replies go, a parent waiting for it continues with `error`, and `decree retry` continues
//! it. An active or finished run, or an unknown id, is refused. Each test builds its own
//! `.decree/` in a temp directory.

use assert_cmd::Command;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

mod common;
use common::write_script;
#[path = "common/schema.rs"]
mod schema;
#[path = "common/traces.rs"]
mod traces;

/// Asks a person; `onexit` logs, so a test sees that cancel runs no script.
const DEPLOY: &str = "\
name: deploy
description: Ask a person to approve, then ship.
initial: approval
states:
  approval:
    description: Ship this build?
    onexit: [record]
    invoke:
      reply:
        question: Ship this build?
        notify: notify_person
    transitions:
      approve: { target: done, description: Ship this build. }
      reject:  { target: done, description: Do not ship. }
      error: done
  done:     { final: true }
  failed:   { final: true }
";

/// Runs `deploy` as a child run, and records which event the child's end gave it.
const PARENT: &str = "\
name: parent
description: Run deploy as a child.
initial: child
states:
  child:
    invoke: { machine: deploy }
    transitions:
      done: done
      error: after_error
  after_error:
    invoke: record
    transitions: { done: done }
  done:   { final: true }
  failed: { final: true }
";

/// Works while `work.flag` is missing; fails while it exists.
const WORK: &str = "\
name: work
description: One step.
initial: step
states:
  step:
    invoke: step
    transitions: { done: done }
  done:   { final: true }
  failed: { final: true }
";

const RECORD: &str =
    "#!/usr/bin/env bash\necho \"$DECREE_STATE\" >> \"$DECREE_PROJECT_ROOT/record.log\"\n";
const STEP: &str = "#!/usr/bin/env bash\n[ ! -e \"$DECREE_PROJECT_ROOT/work.flag\" ]\n";

const RUN: &str = "20260101T000000.000000Z-00000a";

struct Project {
    tmp: TempDir,
}

impl Project {
    fn new() -> Project {
        let tmp = TempDir::new().unwrap();
        let decree = tmp.path().join(".decree");
        for dir in ["inbox", "runs"] {
            fs::create_dir_all(decree.join(dir)).unwrap();
        }
        for (name, text) in [("deploy", DEPLOY), ("parent", PARENT), ("work", WORK)] {
            common::write_file(&decree.join(format!("machines/{name}/machine.yml")), text);
        }
        for (name, text) in [
            ("notify_person", "#!/usr/bin/env bash\n"),
            ("record", RECORD),
            ("step", STEP),
        ] {
            write_script(&decree.join("scripts").join(name), text);
        }
        Project { tmp }
    }

    fn decree(&self) -> PathBuf {
        self.tmp.path().join(".decree")
    }

    /// Queue a message for `machine` as run `id`.
    fn queue(&self, id: &str, machine: &str) {
        common::write_file(
            &self.decree().join(format!("inbox/{id}.md")),
            format!("---\nid: {id}\nmachine: {machine}\n---\nDo it.\n"),
        );
    }

    fn events(&self, id: &str) -> Vec<Value> {
        fs::read_to_string(self.decree().join("runs").join(id).join("events.jsonl"))
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
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

    fn cmd(&self, args: &[&str]) -> Command {
        let mut cmd = common::decree_at(self.tmp.path());
        cmd.env("NO_COLOR", "1").args(args);
        cmd
    }

    fn stdout(&self, args: &[&str], code: i32) -> String {
        let out = self.cmd(args).output().unwrap();
        assert_eq!(out.status.code(), Some(code), "{args:?}: {out:?}");
        String::from_utf8(out.stdout).unwrap()
    }

    fn record(&self) -> String {
        fs::read_to_string(self.tmp.path().join("record.log")).unwrap_or_default()
    }

    fn attention(&self, id: &str) -> PathBuf {
        self.decree().join(format!("attention/{id}.md"))
    }
}

/// Every event of run `id` is valid against `events.schema.json`.
fn assert_valid_events(p: &Project, id: &str) {
    let validator = schema::validator(schema::EVENTS_SCHEMA);
    for event in p.events(id) {
        let errors = schema::errors(&validator, &event);
        assert!(errors.is_empty(), "{errors:?}\n{event:#}");
    }
}

/// A waiting run ends in `failed`, with the note, and no script runs; its attention file and
/// its queued reply go; `status` counts it as `cancelled`; `retry` continues it.
#[test]
fn a_waiting_run_is_cancelled_without_running_a_script() {
    let p = Project::new();
    p.queue(RUN, "deploy");
    p.cmd(&["daemon", "--once"]).assert().success();
    assert!(p.attention(RUN).exists());
    // A reply queued for it would only be rejected once it is cancelled: it goes.
    let reply = p.stdout(&["reply", RUN, "approve"], 0);
    let reply = reply.trim();
    assert!(p.decree().join(format!("inbox/{reply}.md")).exists());

    let out = p.stdout(&["cancel", RUN, "-m", "Not this build."], 0);
    assert!(
        out.contains(&format!("cancelled run {RUN} in `approval`")),
        "{out}"
    );
    let events = p.events(RUN);
    let n = events.len();
    assert_eq!(events[n - 2]["type"], "transition");
    assert_eq!(events[n - 2]["from"], "approval");
    assert_eq!(events[n - 2]["event"], "cancel");
    assert_eq!(events[n - 2]["to"], "failed");
    assert_eq!(events[n - 2]["source"], "cancel");
    assert_eq!(events[n - 2]["note"], "Not this build.");
    assert_eq!(events[n - 1]["type"], "run_finished");
    assert_eq!(events[n - 1]["state"], "failed");
    assert_eq!(events[n - 1]["cancelled"], true);
    assert_valid_events(&p, RUN);
    assert_eq!(p.record(), "", "a cancel runs no script");
    assert!(!p.attention(RUN).exists());
    assert!(!p.decree().join(format!("inbox/{reply}.md")).exists());
    let message = fs::read_to_string(p.decree().join(format!("runs/{RUN}/message.md"))).unwrap();
    assert!(message.contains("\nstate: failed\n"), "{message}");

    // `status` counts it apart, and lists no `retry` line for it.
    let status = p.stdout(&["status"], 0);
    assert!(status.contains("cancelled: 1"), "{status}");
    assert!(!status.contains("decree retry"), "{status}");
    let doc: Value = serde_json::from_str(&p.stdout(&["status", "--format", "json"], 0)).unwrap();
    assert_eq!(doc["runs"]["finished"]["cancelled"][0]["id"], RUN);
    assert_eq!(doc["runs"]["finished"]["cancelled"][0]["state"], "failed");
    let errors = schema::errors(&schema::validator(schema::CLI_STATUS_SCHEMA), &doc);
    assert!(errors.is_empty(), "{errors:?}");

    // A second cancel is refused; a daemon pass does nothing with it.
    p.cmd(&["cancel", RUN]).assert().code(1);
    p.cmd(&["daemon", "--once"]).assert().success();
    assert_eq!(p.events(RUN).len(), n);

    // `retry` continues it in the state it was cancelled from.
    p.stdout(&["retry", RUN], 0);
    p.cmd(&["daemon", "--once"]).assert().success();
    assert_eq!(p.events(RUN).last().unwrap()["type"], "waiting");
}

/// A parent waiting for a child run: cancelling the parent cancels the child too, deepest
/// first. Cancelling only the child gives the parent `error`, as a child that failed does.
#[test]
fn cancel_takes_the_child_runs_and_a_cancelled_child_is_its_parents_error() {
    let p = Project::new();
    p.queue(RUN, "parent");
    p.cmd(&["daemon", "--once"]).assert().success();
    let child = p.runs().into_iter().find(|r| r != RUN).unwrap();
    assert_eq!(p.events(&child).last().unwrap()["type"], "waiting");

    let out = p.stdout(&["cancel", RUN, "--format", "json"], 0);
    let doc: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(doc, json!({ "cancelled": [child, RUN] }));
    let errors = schema::errors(&schema::validator(schema::CLI_CANCEL_SCHEMA), &doc);
    assert!(errors.is_empty(), "{errors:?}");
    for id in [RUN, child.as_str()] {
        assert_eq!(p.events(id).last().unwrap()["cancelled"], true, "{id}");
        assert_valid_events(&p, id);
    }
    p.cmd(&["daemon", "--once"]).assert().success();
    assert_eq!(p.record(), "");

    // Again, cancelling only the child: the parent goes on with `error`.
    let p = Project::new();
    p.queue(RUN, "parent");
    p.cmd(&["daemon", "--once"]).assert().success();
    let child = p.runs().into_iter().find(|r| r != RUN).unwrap();
    let out = p.stdout(&["cancel", &child], 0);
    assert!(
        out.contains(&format!("its parent run {RUN} continues with `error`")),
        "{out}"
    );
    assert!(p.events(RUN).last().unwrap().get("cancelled").is_none());
    p.cmd(&["daemon", "--once"]).assert().success();
    let events = p.events(RUN);
    assert_eq!(events.last().unwrap()["state"], "done");
    assert!(events
        .iter()
        .any(|e| e["type"] == "transition" && e["to"] == "after_error"));
    assert_eq!(p.record(), "after_error\n");
}

/// An interrupted run (a crash) is cancelled; its trace keeps one run span, ended by the
/// `interrupted` event.
#[test]
fn an_interrupted_run_is_cancelled_and_keeps_one_run_span() {
    let p = Project::new();
    p.queue(RUN, "deploy");
    p.cmd(&["daemon", "--once"]).assert().success();
    // A run with a lock whose process is gone reads as crashed; the next start marks it.
    let run_dir = p.decree().join("runs").join(RUN);
    let mut events = fs::read_to_string(run_dir.join("events.jsonl")).unwrap();
    let waiting = events.lines().last().unwrap().to_string();
    events.truncate(events.len() - waiting.len() - 1);
    fs::write(run_dir.join("events.jsonl"), &events).unwrap();
    fs::write(run_dir.join(".lock"), "999999999 dead\n").unwrap();
    p.cmd(&["daemon", "--once"]).assert().success();
    assert_eq!(p.events(RUN).last().unwrap()["type"], "interrupted");

    p.stdout(&["cancel", RUN], 0);
    assert_eq!(p.events(RUN).last().unwrap()["cancelled"], true);
    assert_valid_events(&p, RUN);
    assert!(!p.attention(RUN).exists());
    let spans = traces::spans(&run_dir);
    let runs: Vec<&Value> = spans
        .iter()
        .filter(|s| traces::attr(s, "decree.run_id") == Some(RUN))
        .filter(|s| s["name"].as_str().unwrap_or_default().starts_with("run "))
        .collect();
    assert_eq!(runs.len(), 1, "{spans:#?}");
}

/// Refused, with exit 1 and nothing appended: an unknown id, a finished run, and an active
/// one (its lock holds a live process).
#[test]
fn an_unknown_finished_or_active_run_is_refused() {
    let p = Project::new();
    p.cmd(&["cancel", RUN]).assert().code(1);

    p.queue(RUN, "work");
    p.cmd(&["daemon", "--once"]).assert().success();
    let n = p.events(RUN).len();
    let out = p.cmd(&["cancel", RUN]).assert().code(1);
    let err = String::from_utf8_lossy(&out.get_output().stderr).into_owned();
    assert!(err.contains("has already finished in `done`"), "{err}");
    assert_eq!(p.events(RUN).len(), n);

    let other = "20260101T000000.000001Z-00000b";
    p.queue(other, "deploy");
    p.cmd(&["daemon", "--once"]).assert().success();
    let lock = p.decree().join("runs").join(other).join(".lock");
    fs::write(&lock, format!("{} test\n", std::process::id())).unwrap();
    let n = p.events(other).len();
    let out = p.cmd(&["cancel", other]).assert().code(1);
    let err = String::from_utf8_lossy(&out.get_output().stderr).into_owned();
    assert!(err.contains("is active"), "{err}");
    assert_eq!(p.events(other).len(), n);
    fs::remove_file(&lock).unwrap();
    p.stdout(&["cancel", other], 0);
}

/// A failed run's `retry` names cancel for a waiting run; a pending run (just retried) is
/// cancelled before any daemon continues it.
#[test]
fn a_pending_run_is_cancelled_before_it_continues() {
    let p = Project::new();
    fs::write(p.tmp.path().join("work.flag"), "").unwrap();
    p.queue(RUN, "work");
    p.cmd(&["daemon", "--once"]).assert().code(1);
    p.stdout(&["retry", RUN], 0);
    p.stdout(&["cancel", RUN], 0);
    fs::remove_file(p.tmp.path().join("work.flag")).unwrap();
    p.cmd(&["daemon", "--once"]).assert().success();
    let events = p.events(RUN);
    assert_eq!(events.last().unwrap()["cancelled"], true);
    assert_valid_events(&p, RUN);

    // A waiting run is not retried: the refusal names `reply` and `cancel`.
    let other = "20260101T000000.000001Z-00000b";
    p.queue(other, "deploy");
    p.cmd(&["daemon", "--once"]).assert().success();
    let out = p.cmd(&["retry", other]).assert().code(1);
    let err = String::from_utf8_lossy(&out.get_output().stderr).into_owned();
    assert!(err.contains(&format!("`decree cancel {other}`")), "{err}");
}
