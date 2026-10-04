//! Replies to waiting runs (docs/reference/messages.md, Replies): delivery and rejection when `process`
//! claims a reply, `timeout_s` deadlines, and `decree event`. Each test builds its own
//! `.decree/` in a temp directory.

use assert_cmd::cargo::cargo_bin_cmd;
use assert_cmd::Command;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

mod common;
use common::write_script;

/// Asks a person, then ships or stops; a timeout goes to `expired`.
const DEPLOY: &str = "\
name: deploy
description: Ask a person to approve, then ship.
initial: approval
states:
  approval:
    description: Ship this build?
    invoke:
      person:
        question: Ship this build?
        ask: ask_person
        timeout_s: TIMEOUT
    transitions:
      approve: { target: ship, description: Ship this build. }
      reject:  { target: rejected, description: Do not ship. }
      error: expired
  ship:
    invoke: ship
    transitions: { done: done }
  done:     { final: true }
  rejected: { final: true }
  expired:  { final: true }
  failed:   { final: true }
";

/// Logs the received reply's path, so the test sees `DECREE_RECEIVED`.
const SHIP: &str = r#"#!/usr/bin/env bash
echo "$DECREE_RECEIVED" > "$DECREE_RUN_DIR/shipped.txt"
"#;

const RUN: &str = "run-a";

struct Project {
    tmp: TempDir,
}

impl Project {
    /// A project with one `deploy` message queued as run `run-a`.
    fn new(timeout_s: u64) -> Project {
        let tmp = TempDir::new().unwrap();
        let decree = tmp.path().join(".decree");
        for dir in ["machines", "scripts", "migrations", "inbox", "runs"] {
            fs::create_dir_all(decree.join(dir)).unwrap();
        }
        fs::write(decree.join("processed.md"), "").unwrap();
        let machine = DEPLOY.replace("TIMEOUT", &timeout_s.to_string());
        fs::write(decree.join("machines/deploy.yml"), machine).unwrap();
        for (name, text) in [("ask_person", "#!/usr/bin/env bash\n"), ("ship", SHIP)] {
            write_script(&decree.join("scripts").join(name), text);
        }
        let p = Project { tmp };
        p.write(
            "inbox/a.md",
            &format!("---\nid: {RUN}\nmachine: deploy\n---\nShip it.\n"),
        );
        p
    }

    fn decree(&self) -> PathBuf {
        self.tmp.path().join(".decree")
    }

    fn write(&self, rel: &str, text: &str) {
        fs::write(self.decree().join(rel), text).unwrap();
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

    fn inbox(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(self.decree().join("inbox"))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    fn decree_cmd(&self) -> Command {
        let mut cmd = cargo_bin_cmd!("decree");
        cmd.current_dir(self.tmp.path()).env("NO_COLOR", "1");
        cmd
    }

    fn process(&self) -> Command {
        let mut cmd = self.decree_cmd();
        cmd.arg("process");
        cmd
    }

    fn event(&self, args: &[&str]) -> Command {
        let mut cmd = self.decree_cmd();
        cmd.arg("event").args(args);
        cmd
    }

    /// Run `process` until `run-a` waits; returns its wait id.
    fn wait(&self) -> String {
        let out = self.process().assert().code(0).get_output().clone();
        let last = self.events(RUN).last().unwrap().clone();
        assert_eq!(last["type"], "waiting");
        let wait_id = last["wait_id"].as_str().unwrap().to_string();
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            stdout.contains(&format!("decree event {wait_id} approve")),
            "{stdout}"
        );
        wait_id
    }

    /// The one run other than `run-a`.
    fn other_run(&self) -> String {
        let runs = self.runs();
        assert_eq!(runs.len(), 2, "{runs:?}");
        runs.into_iter().find(|r| r != RUN).unwrap()
    }
}

fn stderr(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn last_transition(events: &[Value]) -> &Value {
    events
        .iter()
        .rev()
        .find(|e| e["type"] == "transition")
        .unwrap()
}

/// Run `id` ended `failed` with `invalid_message`; returns the reason.
fn invalid_message(events: &[Value]) -> String {
    assert_eq!(events.len(), 2, "{events:?}");
    assert_eq!(events[1]["type"], "run_finished");
    assert_eq!(events[1]["state"], "failed");
    let t = &events[0];
    assert_eq!(t["to"], "failed");
    assert_eq!(t["source"], "invalid_message");
    t["error"].as_str().unwrap().to_string()
}

// ---------------------------------------------------------------
// Delivery (docs/reference/messages.md, Replies, steps 2 and 3)
// ---------------------------------------------------------------

#[test]
fn reply_with_wait_id_and_option_continues_with_source_person() {
    let p = Project::new(3600);
    let wait_id = p.wait();
    p.write(
        "inbox/reply.md",
        &format!("---\nto: {wait_id}\nevent: approve\n---\nLooks good.\n"),
    );
    p.process().assert().code(0);

    let events = p.events(RUN);
    let received = events.iter().find(|e| e["type"] == "received").unwrap();
    assert_eq!(received["wait_id"], json!(wait_id));
    assert_eq!(received["event"], "approve");
    assert_eq!(received["file"], "reply.md");
    let decision = events.iter().find(|e| e["type"] == "decision").unwrap();
    assert_eq!(decision["kind"], "person");
    assert_eq!(decision["reply"], "reply.md");
    let taken = events
        .iter()
        .find(|e| e["type"] == "transition" && e["from"] == "approval")
        .unwrap();
    assert_eq!(taken["event"], "approve");
    assert_eq!(taken["source"], "person");
    assert_eq!(last_transition(&events)["to"], "done");

    // The reply is in `received/`, not a run of its own, and later scripts see it.
    let received = p.decree().join("runs").join(RUN).join("received/reply.md");
    assert_eq!(
        fs::read_to_string(&received).unwrap(),
        format!("---\nto: {wait_id}\nevent: approve\n---\nLooks good.\n")
    );
    assert_eq!(p.runs(), [RUN]);
    assert!(p.inbox().is_empty());
    let shipped = fs::read_to_string(p.decree().join("runs").join(RUN).join("shipped.txt"));
    assert_eq!(shipped.unwrap().trim(), received.to_str().unwrap());
}

#[test]
fn reply_naming_the_run_id_answers_its_current_wait() {
    let p = Project::new(3600);
    let wait_id = p.wait();
    p.write(
        "inbox/reply.md",
        &format!("---\nto: {RUN}\nevent: reject\n---\n"),
    );
    p.process().assert().code(0);
    let events = p.events(RUN);
    let received = events.iter().find(|e| e["type"] == "received").unwrap();
    assert_eq!(received["wait_id"], json!(wait_id));
    assert_eq!(last_transition(&events)["to"], "rejected");
    assert_eq!(last_transition(&events)["source"], "person");
}

// ---------------------------------------------------------------
// Rejection (docs/reference/messages.md, Replies, step 4)
// ---------------------------------------------------------------

/// A reply that fails a check: `process` exits 1, `run-a` stays waiting, and the reply
/// is a failed `invalid_message` run whose reason contains `expected`.
fn assert_rejected(p: &Project, reply: &str, expected: &str) {
    let before = p.events(RUN);
    p.write("inbox/reply.md", reply);
    let out = p.process().assert().code(1).get_output().clone();
    assert_eq!(p.events(RUN), before, "the waiting run is unchanged");
    assert_eq!(before.last().unwrap()["type"], "waiting");
    let id = p.other_run();
    let reason = invalid_message(&p.events(&id));
    assert!(reason.contains(expected), "{reason}");
    assert!(stderr(&out).contains(expected), "{}", stderr(&out));
    let message = p.decree().join("runs").join(&id).join("message.md");
    assert!(fs::read_to_string(message)
        .unwrap()
        .contains("state: failed"));
    assert!(!p.decree().join("runs").join(RUN).join("received").exists());
}

#[test]
fn reply_with_stale_wait_id_is_rejected_and_the_run_stays_waiting() {
    let p = Project::new(3600);
    let wait_id = p.wait();
    let reply = format!("---\nto: {RUN}.w999\nevent: approve\n---\n");
    assert_rejected(&p, &reply, &format!("now waits as {wait_id}"));
}

#[test]
fn reply_with_an_event_that_is_not_an_option_is_rejected() {
    let p = Project::new(3600);
    p.wait();
    let reply = format!("---\nto: {RUN}\nevent: maybe\n---\n");
    assert_rejected(&p, &reply, "`maybe` is not an option");
}

#[test]
fn reply_to_an_unknown_run_is_rejected() {
    let p = Project::new(3600);
    p.wait();
    assert_rejected(
        &p,
        "---\nto: nope.w3\nevent: approve\n---\n",
        "names no run",
    );
}

#[test]
fn reply_to_a_finished_run_is_rejected() {
    let p = Project::new(3600);
    p.wait();
    p.write(
        "inbox/reply.md",
        &format!("---\nto: {RUN}\nevent: reject\n---\n"),
    );
    p.process().assert().code(0);
    p.write(
        "inbox/second.md",
        &format!("---\nto: {RUN}\nevent: approve\n---\n"),
    );
    p.process().assert().code(1);
    let id = p.other_run();
    assert!(invalid_message(&p.events(&id)).contains("is not waiting: it is finished"));
}

#[test]
fn reply_never_replaces_an_earlier_reply_of_the_same_name() {
    let p = Project::new(3600);
    p.wait();
    let received = p.decree().join("runs").join(RUN).join("received");
    fs::create_dir_all(&received).unwrap();
    fs::write(received.join("reply.md"), "earlier\n").unwrap();
    assert_rejected_keeping(&p, "already received a reply named reply.md");
    assert_eq!(
        fs::read_to_string(received.join("reply.md")).unwrap(),
        "earlier\n"
    );
}

/// `assert_rejected` for a project whose `received/` already exists.
fn assert_rejected_keeping(p: &Project, expected: &str) {
    let before = p.events(RUN);
    p.write(
        "inbox/reply.md",
        &format!("---\nto: {RUN}\nevent: approve\n---\n"),
    );
    p.process().assert().code(1);
    assert_eq!(p.events(RUN), before);
    let reason = invalid_message(&p.events(&p.other_run()));
    assert!(reason.contains(expected), "{reason}");
}

// ---------------------------------------------------------------
// Timeout (docs/reference/messages.md, Replies, step 5)
// ---------------------------------------------------------------

#[test]
fn timeout_after_the_deadline_continues_with_received_error_timed_out() {
    let p = Project::new(1);
    let wait_id = p.wait();
    let waiting = p.events(RUN);
    std::thread::sleep(std::time::Duration::from_millis(1100));
    p.process().assert().code(0);

    let events = p.events(RUN);
    let received = &events[waiting.len()];
    assert_eq!(received["type"], "received");
    assert_eq!(received["wait_id"], json!(wait_id));
    assert_eq!(received["event"], "error");
    assert_eq!(received["timed_out"], true);
    assert!(received.get("file").is_none());
    let taken = last_transition(&events);
    assert_eq!(taken["from"], "approval");
    assert_eq!(taken["event"], "error");
    assert_eq!(taken["to"], "expired");
    assert_eq!(taken["source"], "timeout");
}

#[test]
fn no_timeout_before_the_deadline() {
    let p = Project::new(3600);
    p.wait();
    let before = p.events(RUN);
    p.process().assert().code(0);
    assert_eq!(p.events(RUN), before);
}

// ---------------------------------------------------------------
// decree event (docs/reference/cli.md)
// ---------------------------------------------------------------

#[test]
fn event_queues_a_reply_that_process_delivers() {
    let p = Project::new(3600);
    let wait_id = p.wait();
    let out = p
        .event(&[&wait_id, "approve", "-m", "Ship it."])
        .assert()
        .code(0)
        .get_output()
        .clone();
    let id = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert_eq!(p.inbox(), [format!("{id}.md")]);
    let text = fs::read_to_string(p.decree().join("inbox").join(format!("{id}.md"))).unwrap();
    assert_eq!(
        text,
        format!("---\nid: {id}\nto: {wait_id}\nevent: approve\n---\nShip it.\n")
    );
    p.process().assert().code(0);
    let events = p.events(RUN);
    assert_eq!(last_transition(&events)["to"], "done");
    let received = p.decree().join("runs").join(RUN).join("received");
    assert!(received.join(format!("{id}.md")).is_file());
}

#[test]
fn event_for_a_run_that_is_not_waiting_exits_1() {
    let p = Project::new(3600);
    p.wait();
    p.event(&[RUN, "reject"]).assert().code(0);
    p.process().assert().code(0);
    let out = p
        .event(&[RUN, "approve"])
        .assert()
        .code(1)
        .get_output()
        .clone();
    assert!(stderr(&out).contains("is not waiting"), "{}", stderr(&out));
    assert!(p.inbox().is_empty());
}

#[test]
fn event_with_an_unaccepted_event_or_stale_or_unknown_target_exits_1() {
    let p = Project::new(3600);
    let wait_id = p.wait();
    for (args, expected) in [
        (vec![wait_id.as_str(), "maybe"], "is not an option"),
        (vec!["run-a.w999", "approve"], "stale wait id"),
        (vec!["nope", "approve"], "names no run"),
    ] {
        let out = p.event(&args).assert().code(1).get_output().clone();
        assert!(stderr(&out).contains(expected), "{}", stderr(&out));
    }
    assert!(p.inbox().is_empty());
}
