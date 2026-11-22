//! Replies to waiting runs (docs/reference/messages.md, Replies): delivery and rejection when `process`
//! claims a reply, `timeout` deadlines, and `decree reply`. Each test builds its own
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

/// Asks a person, then ships or stops; a timeout goes to `expired`.
const DEPLOY: &str = "\
name: deploy
description: Ask a person to approve, then ship.
initial: approval
states:
  approval:
    description: Ship this build?
    invoke:
      reply:
        question: Ship this build?
        notify: ask_person
        timeout: TIMEOUT
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

const RUN: &str = "20260101T000000.000000Z-00000a";

struct Project {
    tmp: TempDir,
}

impl Project {
    /// A project with one `deploy` message queued as run `RUN`.
    fn new(timeout: &str) -> Project {
        let tmp = TempDir::new().unwrap();
        let decree = tmp.path().join(".decree");
        for dir in ["machines", "scripts", "inbox", "runs"] {
            fs::create_dir_all(decree.join(dir)).unwrap();
        }
        fs::write(decree.join("processed.md"), "").unwrap();
        let machine = DEPLOY.replace("TIMEOUT", timeout);
        common::write_file(&decree.join("machines/deploy/machine.yml"), machine);
        for (name, text) in [("ask_person", "#!/usr/bin/env bash\n"), ("ship", SHIP)] {
            write_script(&decree.join("scripts").join(name), text);
        }
        let p = Project { tmp };
        p.write(
            &format!("inbox/{RUN}.md"),
            &format!("---\nid: {RUN}\nmachine: deploy\n---\nShip it.\n"),
        );
        p
    }

    fn decree(&self) -> PathBuf {
        self.tmp.path().join(".decree")
    }

    fn write(&self, rel: &str, text: &str) {
        common::write_file(&self.decree().join(rel), text);
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
        let mut cmd = common::decree_at(self.tmp.path());
        cmd.env("NO_COLOR", "1");
        cmd
    }

    fn process(&self) -> Command {
        let mut cmd = self.decree_cmd();
        cmd.args(["daemon", "--once"]);
        cmd
    }

    fn event(&self, args: &[&str]) -> Command {
        let mut cmd = self.decree_cmd();
        cmd.arg("reply").args(args);
        cmd
    }

    /// Run `process` until `RUN` waits; returns its wait id.
    fn wait(&self) -> String {
        let out = self.process().assert().code(0).get_output().clone();
        let last = self.events(RUN).last().unwrap().clone();
        assert_eq!(last["type"], "waiting");
        let wait_id = last["wait_id"].as_str().unwrap().to_string();
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            stdout.contains(&format!("decree reply {wait_id} approve")),
            "{stdout}"
        );
        wait_id
    }

    /// The one run other than `RUN`.
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
fn reply_with_wait_id_and_option_continues_with_source_reply() {
    let p = Project::new("1h");
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
    // The reply got an id when it arrived; `received/` holds it under that name.
    let file = received["file"].as_str().unwrap().to_string();
    let reply_id = file.strip_suffix(".md").unwrap();
    assert!(
        reply_id.len() == 30 && reply_id.as_bytes()[8] == b'T',
        "{file}"
    );
    let decision = events.iter().find(|e| e["type"] == "decision").unwrap();
    assert_eq!(decision["kind"], "reply");
    assert_eq!(decision["reply"], json!(file));
    let taken = events
        .iter()
        .find(|e| e["type"] == "transition" && e["from"] == "approval")
        .unwrap();
    assert_eq!(taken["event"], "approve");
    assert_eq!(taken["source"], "reply");
    assert_eq!(last_transition(&events)["to"], "done");

    // The reply is in `received/`, not a run of its own, and later scripts see it.
    let received = p
        .decree()
        .join("runs")
        .join(RUN)
        .join("received")
        .join(&file);
    assert_eq!(
        fs::read_to_string(&received).unwrap(),
        format!("---\nid: {reply_id}\narrived_as: reply.md\nto: {wait_id}\nevent: approve\n---\nLooks good.\n")
    );
    assert_eq!(p.runs(), [RUN]);
    assert!(p.inbox().is_empty());
    let shipped = fs::read_to_string(p.decree().join("runs").join(RUN).join("shipped.txt"));
    assert_eq!(shipped.unwrap().trim(), received.to_str().unwrap());
}

#[test]
fn reply_naming_the_run_id_answers_its_current_wait() {
    let p = Project::new("1h");
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
    assert_eq!(last_transition(&events)["source"], "reply");
}

// ---------------------------------------------------------------
// Rejection (docs/reference/messages.md, Replies, step 4)
// ---------------------------------------------------------------

/// A reply that fails a check: `process` exits 1, `RUN` stays waiting, and the reply
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
    let p = Project::new("1h");
    let wait_id = p.wait();
    let reply = format!("---\nto: {RUN}.w999\nevent: approve\n---\n");
    assert_rejected(&p, &reply, &format!("now waits as {wait_id}"));
}

#[test]
fn reply_with_an_event_that_is_not_an_option_is_rejected() {
    let p = Project::new("1h");
    p.wait();
    let reply = format!("---\nto: {RUN}\nevent: maybe\n---\n");
    assert_rejected(&p, &reply, "`maybe` is not an option");
}

#[test]
fn reply_to_an_unknown_run_is_rejected() {
    let p = Project::new("1h");
    p.wait();
    assert_rejected(
        &p,
        "---\nto: nope.w3\nevent: approve\n---\n",
        "names no run",
    );
}

#[test]
fn reply_to_a_finished_run_is_rejected() {
    let p = Project::new("1h");
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
    let p = Project::new("1h");
    p.wait();
    let name = "20260101T000000.000001Z-00000b.md";
    let received = p.decree().join("runs").join(RUN).join("received");
    fs::create_dir_all(&received).unwrap();
    fs::write(received.join(name), "earlier\n").unwrap();
    let before = p.events(RUN);
    // Queued as `decree reply` writes it, so it keeps its name.
    p.write(
        &format!("inbox/{name}"),
        &format!(
            "---\nid: {}\nto: {RUN}\nevent: approve\n---\n",
            name.strip_suffix(".md").unwrap()
        ),
    );
    p.process().assert().code(1);
    assert_eq!(p.events(RUN), before);
    let reason = invalid_message(&p.events(&p.other_run()));
    let expected = format!("already received a reply named {name}");
    assert!(reason.contains(&expected), "{reason}");
    assert_eq!(
        fs::read_to_string(received.join(name)).unwrap(),
        "earlier\n"
    );
}

/// Replies are ordered as other messages: one written by hand as `01-reply.md` after
/// `decree reply` queued one runs after it, though its name sorts first.
#[test]
fn a_reply_written_after_a_queued_one_is_delivered_after_it() {
    let p = Project::new("1h");
    let wait_id = p.wait();
    p.event(&[&wait_id, "reject"]).assert().code(0);
    p.write(
        "inbox/01-reply.md",
        &format!("---\nto: {RUN}\nevent: approve\n---\n"),
    );
    p.process().assert().code(1);
    let events = p.events(RUN);
    assert_eq!(last_transition(&events)["to"], "rejected");
    let id = p.other_run();
    assert!(invalid_message(&p.events(&id)).contains("is not waiting: it is finished"));
    let message = fs::read_to_string(p.decree().join("runs").join(&id).join("message.md")).unwrap();
    assert!(message.contains("arrived_as: 01-reply.md\n"), "{message}");
}

// ---------------------------------------------------------------
// Timeout (docs/reference/messages.md, Replies, step 5)
// ---------------------------------------------------------------

#[test]
fn timeout_after_the_deadline_continues_with_received_error_timed_out() {
    let p = Project::new("1s");
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
    let p = Project::new("1h");
    p.wait();
    let before = p.events(RUN);
    p.process().assert().code(0);
    assert_eq!(p.events(RUN), before);
}

// ---------------------------------------------------------------
// decree reply (docs/reference/cli.md)
// ---------------------------------------------------------------

#[test]
fn event_queues_a_reply_that_process_delivers() {
    let p = Project::new("1h");
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
    let p = Project::new("1h");
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
    let p = Project::new("1h");
    let wait_id = p.wait();
    for (args, expected) in [
        (vec![wait_id.as_str(), "maybe"], "is not an option"),
        (vec![&format!("{RUN}.w999"), "approve"], "stale wait id"),
        (vec!["nope", "approve"], "names no run"),
    ] {
        let out = p.event(&args).assert().code(1).get_output().clone();
        assert!(stderr(&out).contains(expected), "{}", stderr(&out));
    }
    assert!(p.inbox().is_empty());
}

#[test]
fn event_while_a_reply_is_already_queued_exits_1() {
    let p = Project::new("1h");
    let wait_id = p.wait();
    let out = p
        .event(&[&wait_id, "approve"])
        .assert()
        .code(0)
        .get_output()
        .clone();
    let first = format!("{}.md", String::from_utf8_lossy(&out.stdout).trim());
    // A second reply to the same wait id could never be delivered: refused.
    let out = p
        .event(&[&wait_id, "reject"])
        .assert()
        .code(1)
        .get_output()
        .clone();
    let expected = format!("wait {wait_id} already has a reply queued: .decree/inbox/{first}");
    assert!(stderr(&out).contains(&expected), "{}", stderr(&out));
    // One to the run id goes to the wait the run is in when it is delivered, so it is
    // queued after the first; here the first finishes the run, and it is rejected then.
    let out = p.event(&[RUN, "reject"]).assert().code(0).get_output().clone();
    let second = format!("{}.md", String::from_utf8_lossy(&out.stdout).trim());
    assert_eq!(p.inbox(), [first, second]);
    // `--once` exits 1 for the rejected reply's failed run, which names why.
    p.process().assert().code(1);
    assert_eq!(last_transition(&p.events(RUN))["to"], "done");
    let rejected = p.other_run();
    assert!(invalid_message(&p.events(&rejected)).contains("is not waiting"));
}

/// AC: a reply to a run id may be queued while the run is busy (pending, or executing in
/// another process), when a `reply` state of its machine takes the event; one no state
/// takes is refused.
#[test]
fn a_reply_to_a_busy_run_is_queued_for_its_next_wait() {
    let p = Project::new("1h");
    p.wait();
    // Pending: a reply was delivered and the run not yet continued.
    p.event(&[RUN, "approve"]).assert().code(0);
    let decree = p.decree();
    let events = decree.join("runs").join(RUN).join("events.jsonl");
    let mut log = fs::read_to_string(&events).unwrap();
    let wait_id = p.events(RUN).last().unwrap()["wait_id"].as_str().unwrap().to_string();
    log.push_str(&format!(
        "{}\n",
        json!({"type": "received", "run_id": RUN, "machine": "deploy", "seq": 99, "wait_id": wait_id, "event": "approve", "ts": "2026-01-01T00:00:00Z"})
    ));
    fs::write(&events, log).unwrap();
    let out = p.event(&[RUN, "nope"]).assert().code(1).get_output().clone();
    assert!(stderr(&out).contains("no `reply` state of machine `deploy` takes `nope`"), "{}", stderr(&out));
    p.event(&[RUN, "reject"]).assert().code(0);
}

// ---------------------------------------------------------------
// A reply queued before the deadline wins (docs/reference/messages.md, Replies, step 5);
// a rejected reply never holds the queue (step 4)
// ---------------------------------------------------------------

/// The deadline the tests set on `RUN`'s wait, and reply ids queued before and after it.
const DEADLINE: &str = "2026-01-01T00:00:10.000Z";
const BEFORE: &str = "20260101T000005.000000Z-0000b1";
const AFTER: &str = "20260101T000015.000000Z-0000b2";

impl Project {
    /// Set the `timeout_at` of `RUN`'s wait, its last event, to `at`.
    fn set_deadline(&self, at: &str) {
        let path = self.decree().join("runs").join(RUN).join("events.jsonl");
        let text = fs::read_to_string(&path).unwrap();
        let (head, last) = text.trim_end().rsplit_once('\n').unwrap();
        let key = "\"timeout_at\":\"";
        let start = last.find(key).unwrap() + key.len();
        let end = start + last[start..].find('"').unwrap();
        let last = format!("{}{at}{}", &last[..start], &last[end..]);
        fs::write(&path, format!("{head}\n{last}\n")).unwrap();
    }

    /// Queue a reply as `decree reply` does: `inbox/<id>.md`, with frontmatter `id`.
    fn queue_reply(&self, id: &str, to: &str, event: &str) {
        self.write(
            &format!("inbox/{id}.md"),
            &format!("---\nid: {id}\nto: {to}\nevent: {event}\n---\n"),
        );
    }

    /// The frontmatter of run `id`'s attention file, checked against `attention.schema.json`.
    fn attention(&self, id: &str) -> Value {
        let path = self.decree().join("attention").join(format!("{id}.md"));
        let text = fs::read_to_string(&path).unwrap();
        let json = schema::frontmatter_to_json(&text).unwrap();
        let validator = schema::validator(schema::ATTENTION_SCHEMA);
        assert_eq!(
            schema::errors(&validator, &json),
            Vec::<String>::new(),
            "{text}"
        );
        json
    }

    /// `decree status --format json`, checked against its schema.
    fn status(&self) -> Value {
        let out = self
            .decree_cmd()
            .args(["status", "--format", "json"])
            .assert()
            .code(0)
            .get_output()
            .clone();
        let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
        let validator = schema::validator(schema::CLI_STATUS_SCHEMA);
        assert_eq!(schema::errors(&validator, &doc), Vec::<String>::new());
        doc
    }
}

fn timed_out(events: &[Value]) -> bool {
    events.iter().any(|e| e.get("timed_out").is_some())
}

/// AC: a reply to the wait id queued before the deadline is delivered, not the timeout.
#[test]
fn a_reply_queued_before_the_deadline_wins_over_the_timeout() {
    let p = Project::new("1h");
    let wait_id = p.wait();
    p.set_deadline(DEADLINE);
    p.queue_reply(BEFORE, &wait_id, "approve");
    p.process().assert().code(0);
    let events = p.events(RUN);
    assert!(!timed_out(&events), "{events:?}");
    let answered = events
        .iter()
        .find(|e| e["type"] == "transition" && e["from"] == "approval")
        .unwrap();
    assert_eq!(answered["event"], "approve");
    assert_eq!(answered["source"], "reply");
    assert_eq!(last_transition(&events)["to"], "done");
    let received = p.decree().join("runs").join(RUN).join("received");
    assert!(received.join(format!("{BEFORE}.md")).is_file());
    assert_eq!(p.runs(), [RUN]);
}

/// AC: a reply to the run id, its current wait, queued before the deadline also wins.
#[test]
fn a_reply_to_the_run_id_queued_before_the_deadline_wins() {
    let p = Project::new("1h");
    p.wait();
    p.set_deadline(DEADLINE);
    p.queue_reply(BEFORE, RUN, "reject");
    p.process().assert().code(0);
    let events = p.events(RUN);
    assert!(!timed_out(&events), "{events:?}");
    assert_eq!(last_transition(&events)["to"], "rejected");
    assert_eq!(last_transition(&events)["source"], "reply");
}

/// A file written straight into `inbox/` gets its id, the time that counts, when decree
/// first sees it: after the deadline here, so the timeout wins.
#[test]
fn a_reply_written_by_hand_counts_from_when_decree_gives_it_an_id() {
    let p = Project::new("1h");
    p.wait();
    p.set_deadline(DEADLINE);
    p.write(
        "inbox/reply.md",
        &format!("---\nto: {RUN}\nevent: approve\n---\n"),
    );
    p.process().assert().code(1);
    let events = p.events(RUN);
    assert!(timed_out(&events), "{events:?}");
    assert_eq!(last_transition(&events)["to"], "expired");
}

/// AC: a reply queued after the deadline loses: the run times out, and the reply is a
/// failed `invalid_message` run whose attention file does not hold.
#[test]
fn a_reply_queued_after_the_deadline_loses_and_does_not_hold() {
    let p = Project::new("1h");
    let wait_id = p.wait();
    p.set_deadline(DEADLINE);
    p.queue_reply(AFTER, &wait_id, "approve");
    p.process().assert().code(1);
    let events = p.events(RUN);
    assert!(timed_out(&events), "{events:?}");
    let taken = last_transition(&events);
    assert_eq!(taken["event"], "error");
    assert_eq!(taken["to"], "expired");
    assert_eq!(taken["source"], "timeout");
    let id = p.other_run();
    assert_eq!(id, AFTER);
    assert!(invalid_message(&p.events(&id)).contains("is not waiting: it is finished"));
    let attention = p.attention(&id);
    assert_eq!(attention["holds"], false);
    assert_eq!(attention["machine"], "deploy");
}

/// AC: a reply queued before the deadline that names an unknown option is rejected, and
/// the run takes the timeout.
#[test]
fn an_early_reply_with_an_unknown_option_is_rejected_and_the_timeout_delivered() {
    let p = Project::new("1h");
    let wait_id = p.wait();
    p.set_deadline(DEADLINE);
    p.queue_reply(BEFORE, &wait_id, "maybe");
    p.process().assert().code(1);
    let reason = invalid_message(&p.events(BEFORE));
    assert!(reason.contains("`maybe` is not an option"), "{reason}");
    let events = p.events(RUN);
    assert!(timed_out(&events), "{events:?}");
    assert_eq!(last_transition(&events)["to"], "expired");
    assert_eq!(p.attention(BEFORE)["holds"], false);
}

/// Of two replies queued before the deadline, the first queued is delivered; the second,
/// for a run no longer waiting, is rejected.
#[test]
fn of_two_early_replies_the_first_queued_wins() {
    let p = Project::new("1h");
    let wait_id = p.wait();
    p.set_deadline(DEADLINE);
    p.queue_reply("20260101T000006.000000Z-0000b3", RUN, "approve");
    p.queue_reply(BEFORE, &wait_id, "reject");
    p.process().assert().code(1);
    let events = p.events(RUN);
    assert!(!timed_out(&events), "{events:?}");
    assert_eq!(last_transition(&events)["to"], "rejected");
    let reason = invalid_message(&p.events("20260101T000006.000000Z-0000b3"));
    assert!(reason.contains("is not waiting"), "{reason}");
}

/// AC: replies to a finished run and to a run id that does not exist each become a failed
/// `invalid_message` run whose attention file is valid and does not hold; `status` lists
/// them, nothing holds, and a message queued behind them runs.
#[test]
fn rejected_replies_never_hold_the_queue() {
    let p = Project::new("1h");
    p.wait();
    p.event(&[RUN, "reject"]).assert().code(0);
    p.process().assert().code(0);
    assert_eq!(last_transition(&p.events(RUN))["to"], "rejected");

    p.write(
        "machines/note/machine.yml",
        "name: note\ndescription: One script, then done.\ninitial: work\nstates:\n  \
         work:\n    invoke: ask_person\n    transitions: { done: done }\n  \
         done: { final: true }\n  failed: { final: true }\n",
    );
    let (finished, unknown) = (BEFORE, AFTER);
    p.queue_reply(finished, RUN, "approve");
    p.queue_reply(unknown, "20250101T000000.000000Z-0000ff.w1", "approve");
    p.write("inbox/zz-note.md", "---\nmachine: note\n---\n# Note\n");
    let out = p.process().assert().code(1).get_output().clone();

    assert!(invalid_message(&p.events(finished)).contains("is not waiting: it is finished"));
    assert!(invalid_message(&p.events(unknown)).contains("names no run"));
    let finished_file = p.attention(finished);
    assert_eq!(finished_file["holds"], false);
    assert_eq!(finished_file["machine"], "deploy");
    assert_eq!(finished_file["status"], "failed");
    let unknown_file = p.attention(unknown);
    assert_eq!(unknown_file["holds"], false);
    assert!(unknown_file.get("machine").is_none(), "{unknown_file}");

    let note = common::run_of(&p.decree(), "zz-note.md");
    assert_eq!(
        last_transition(&p.events(&note))["to"],
        "done",
        "{}",
        stderr(&out)
    );

    let doc = p.status();
    let attention = doc["attention"].as_array().unwrap();
    let paths: Vec<&str> = attention
        .iter()
        .map(|a| a["path"].as_str().unwrap())
        .collect();
    for id in [finished, unknown] {
        assert!(
            paths.contains(&format!(".decree/attention/{id}.md").as_str()),
            "{paths:?}"
        );
    }
    assert!(
        attention.iter().all(|a| a["holds"] == false),
        "{attention:?}"
    );
    let text = String::from_utf8_lossy(
        &p.decree_cmd()
            .arg("status")
            .assert()
            .code(0)
            .get_output()
            .stdout,
    )
    .into_owned();
    assert!(!text.contains("holds the queue"), "{text}");
    assert!(
        text.contains(&format!("run {unknown} failed in `failed`")),
        "{text}"
    );
}

/// AC: `decree reply` before a 1s deadline, and a daemon whose first pass starts after it:
/// the reply is taken, and nothing holds the daemon.
#[test]
fn daemon_takes_a_reply_sent_before_the_deadline() {
    use std::time::{Duration, Instant};
    let p = Project::new("1s");
    let wait_id = p.wait();
    p.event(&[&wait_id, "approve"]).assert().code(0);
    let deadline = p.events(RUN).last().unwrap()["timeout_at"]
        .as_str()
        .unwrap()
        .to_string();
    let deadline = chrono::DateTime::parse_from_rfc3339(&deadline).unwrap();
    while chrono::Utc::now() <= deadline {
        std::thread::sleep(Duration::from_millis(20));
    }

    let mut daemon = common::std_decree_at(p.tmp.path())
        .env("NO_COLOR", "1")
        .env_remove("DECREE_MACHINES")
        .env_remove("DECREE_EXCLUDE_MACHINES")
        .args(["daemon", "--interval", "1s"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let limit = Instant::now() + Duration::from_secs(30);
    while p.events(RUN).last().unwrap()["type"] != "run_finished" {
        assert!(Instant::now() < limit, "the run did not finish");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(daemon.try_wait().unwrap().is_none(), "the daemon stopped");
    // SAFETY: sends SIGTERM to the daemon this test spawned.
    unsafe { libc::kill(daemon.id() as i32, libc::SIGTERM) };
    let limit = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(status) = daemon.try_wait().unwrap() {
            assert_eq!(status.code(), Some(0));
            break;
        }
        assert!(Instant::now() < limit, "daemon did not exit");
        std::thread::sleep(Duration::from_millis(20));
    }

    let events = p.events(RUN);
    assert!(!timed_out(&events), "{events:?}");
    assert_eq!(last_transition(&events)["to"], "done");
    assert_eq!(p.runs(), [RUN]);
    let attention = p.status()["attention"].as_array().unwrap().clone();
    assert!(
        attention.iter().all(|a| a["holds"] == false),
        "{attention:?}"
    );
}

/// AC: `daemon --once` sleeping until a timer (a wait with `attention: false` and a deadline a
/// day away) wakes when a reply arrives in `inbox/`, delivers it and ends, without waiting
/// for the deadline.
#[test]
fn once_sleeping_on_a_timer_delivers_a_reply_that_arrives() {
    let p = Project::new("1d");
    let machine = DEPLOY
        .replace("TIMEOUT", "1d")
        .replace("timeout: 1d", "timeout: 1d\n        attention: false");
    common::write_file(&p.decree().join("machines/deploy/machine.yml"), machine);
    let mut once = common::std_decree_at(p.tmp.path())
        .args(["daemon", "--once"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    // Once the run waits, reply.
    let start = std::time::Instant::now();
    let events = p.decree().join("runs").join(RUN).join("events.jsonl");
    while !fs::read_to_string(&events).is_ok_and(|t| t.contains("\"type\":\"waiting\"")) {
        assert!(start.elapsed() < std::time::Duration::from_secs(20), "never waited");
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    p.event(&[RUN, "approve"]).assert().code(0);
    let replied = std::time::Instant::now();
    loop {
        if let Some(status) = once.try_wait().unwrap() {
            assert!(status.success(), "{status}");
            break;
        }
        if replied.elapsed() > std::time::Duration::from_secs(15) {
            let _ = once.kill();
            panic!("daemon --once kept sleeping until the timer");
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert_eq!(last_transition(&p.events(RUN))["to"], "done");
}
