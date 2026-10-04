//! Failure scenarios through the binary (docs/reference/runs.md, scripts.md, messages.md):
//! signals and `decree retry`, two `decree process` at once, router replies decree
//! rejects or doubts, `max_depth`, `onexit` failures and the log cap. Scenarios other files
//! already cover: SIGKILL and the stale lock (`interrupt_test.rs`), stale or wrong replies
//! and `timeout_s` (`reply_test.rs`), a failed migration (`process_test.rs`, rule 4). Each
//! test builds its own `.decree/` in a temp directory.

use assert_cmd::cargo::cargo_bin_cmd;
use assert_cmd::Command;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

mod common;
use common::write_script;

/// Root and state hooks around an invoke that waits while `wait.flag` exists.
const SLOW: &str = "\
name: slow
description: Hooks around an invoke that waits while wait.flag exists.
onentry: [record]
onexit: [record]
initial: work
states:
  work:
    onentry: [record]
    invoke: work
    onexit: [record]
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
";

/// Appends `<state> <phase> <script>` to `order.log`.
const RECORD: &str = r#"#!/usr/bin/env bash
echo "$DECREE_STATE $DECREE_PHASE $(basename "$0")" >> "$DECREE_PROJECT_ROOT/order.log"
"#;

/// Records itself, writes its pid (the leader of its process group), then waits while
/// `wait.flag` exists.
const WORK: &str = r#"#!/usr/bin/env bash
echo "$DECREE_STATE $DECREE_PHASE work" >> "$DECREE_PROJECT_ROOT/order.log"
echo "$$" > "$DECREE_RUN_DIR/work.pid.tmp" && mv "$DECREE_RUN_DIR/work.pid.tmp" "$DECREE_RUN_DIR/work.pid"
while [ -e "$DECREE_PROJECT_ROOT/wait.flag" ]; do sleep 0.02; done
"#;

/// A choice a router makes; below 0.8 it is `unsure`.
const PICK: &str = "\
name: pick
description: Let a router pick.
initial: decide
states:
  decide:
    invoke:
      model:
        question: Ship or rework?
        min_confidence: 0.8
    transitions:
      ship: { target: shipped, description: Ship it. }
      rework: { target: reworked, description: Rework it. }
      unsure: unsure_end
  shipped: { final: true }
  reworked: { final: true }
  unsure_end: { final: true }
  failed: { final: true }
";

/// The default router: its script writes `reply.json` from the project root.
const ROUTER: &str = "\
name: router
description: Reply with the project's reply.json.
initial: ask
states:
  ask:
    invoke: reply
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
";

const REPLY: &str = r#"#!/usr/bin/env bash
cp "$DECREE_PROJECT_ROOT/reply.json" "$DECREE_REPLY"
"#;

/// Invokes `leaf` as a child run.
const NEST: &str = "\
name: nest
description: Run leaf as a child run.
initial: sub
states:
  sub:
    invoke: { machine: leaf }
    transitions: { done: done, error: errored }
  done: { final: true }
  errored: { final: true }
  failed: { final: true }
";

const LEAF: &str = "\
name: leaf
description: Run one script.
initial: work
states:
  work:
    invoke: record
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
";

/// `work`'s first `onexit` script fails; the second still runs.
const EXITS: &str = "\
name: exits
description: A failing onexit script.
initial: work
states:
  work:
    invoke: record
    onexit: [fail_exit, record]
    transitions: { done: next }
  next:
    invoke: record
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
";

const FAIL_EXIT: &str = "#!/usr/bin/env bash\necho failing >&2\nexit 3\n";

/// Prints `$DECREE_DATA_LINES` lines of 16 bytes.
const LOUD: &str = "\
name: loud
description: Print a lot.
data:
  lines: { type: int, default: 1 }
initial: work
states:
  work:
    invoke: big
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
";

const BIG: &str = r#"#!/usr/bin/env bash
yes 0123456789abcde | head -n "$DECREE_DATA_LINES"
"#;

struct Project {
    tmp: TempDir,
}

impl Project {
    fn new(machines: &[(&str, &str)], scripts: &[(&str, &str)]) -> Project {
        let tmp = TempDir::new().unwrap();
        let decree = tmp.path().join(".decree");
        for dir in ["machines", "scripts", "migrations", "inbox", "runs"] {
            fs::create_dir_all(decree.join(dir)).unwrap();
        }
        fs::write(decree.join("processed.md"), "").unwrap();
        for (name, text) in machines {
            fs::write(decree.join(format!("machines/{name}.yml")), text).unwrap();
        }
        for (name, text) in scripts {
            write_script(&decree.join("scripts").join(name), text);
        }
        Project { tmp }
    }

    fn root(&self) -> &Path {
        self.tmp.path()
    }

    fn run_dir(&self, id: &str) -> PathBuf {
        self.root().join(".decree/runs").join(id)
    }

    fn queue(&self, name: &str, text: &str) {
        fs::write(self.root().join(".decree/inbox").join(name), text).unwrap();
    }

    fn events(&self, id: &str) -> Vec<Value> {
        fs::read_to_string(self.run_dir(id).join("events.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    fn of_type(&self, id: &str, kind: &str) -> Vec<Value> {
        self.events(id)
            .into_iter()
            .filter(|e| e["type"] == kind)
            .collect()
    }

    fn runs(&self) -> Vec<String> {
        let mut runs: Vec<String> = fs::read_dir(self.root().join(".decree/runs"))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        runs.sort();
        runs
    }

    fn order(&self) -> Vec<String> {
        fs::read_to_string(self.root().join("order.log"))
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect()
    }

    fn cmd(&self, args: &[&str]) -> Command {
        let mut cmd = cargo_bin_cmd!("decree");
        cmd.current_dir(self.root()).env("NO_COLOR", "1").args(args);
        cmd
    }

    fn spawn_process(&self) -> Child {
        std::process::Command::new(env!("CARGO_BIN_EXE_decree"))
            .current_dir(self.root())
            .env("NO_COLOR", "1")
            .arg("process")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap()
    }

    /// Start `decree process` with `wait.flag` set and wait until run `id`'s `work`
    /// script runs. Returns decree and the script's pid, its process group.
    fn working(&self, id: &str) -> (Child, i32) {
        fs::write(self.root().join("wait.flag"), "").unwrap();
        let decree = self.spawn_process();
        let pid = self.run_dir(id).join("work.pid");
        let deadline = Instant::now() + Duration::from_secs(30);
        while !pid.exists() {
            assert!(Instant::now() < deadline, "work never started");
            thread::sleep(Duration::from_millis(5));
        }
        let pid = fs::read_to_string(pid).unwrap().trim().parse().unwrap();
        (decree, pid)
    }
}

fn wait_with_timeout(child: &mut Child, timeout: Duration) -> std::process::ExitStatus {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        assert!(Instant::now() < deadline, "decree did not exit in time");
        thread::sleep(Duration::from_millis(5));
    }
}

fn group_alive(pgid: i32) -> bool {
    // SAFETY: signal 0 to a process group only checks that a member exists.
    unsafe { libc::kill(-pgid, 0) == 0 }
}

fn transitions(events: &[Value]) -> Vec<String> {
    events
        .iter()
        .filter(|e| e["type"] == "transition")
        .map(|e| format!("{} {} {} {}", e["from"], e["event"], e["to"], e["source"]))
        .collect()
}

// ---------------------------------------------------------------
// Signals (docs/reference/messages.md, Stopping; cli.md, `retry`)
// ---------------------------------------------------------------

#[test]
fn sigterm_during_a_script_interrupts_the_run_and_decree_retry_reruns_the_script() {
    let p = Project::new(&[("slow", SLOW)], &[("record", RECORD), ("work", WORK)]);
    p.queue("a.md", "---\nid: run-a\nmachine: slow\n---\n");
    let (mut decree, pgid) = p.working("run-a");
    // SAFETY: sends SIGTERM to the decree process.
    unsafe { libc::kill(decree.id() as i32, libc::SIGTERM) };
    let status = wait_with_timeout(&mut decree, Duration::from_secs(15));
    assert_eq!(status.code(), Some(130));

    let last = p.events("run-a").last().unwrap().clone();
    assert_eq!(last["type"], "interrupted");
    assert_eq!(last["cause"], "signal");
    assert_eq!(last["state"], "work");
    assert_eq!(last["script"], "work");
    // The script's process group is gone (a zombie may wait a moment for its reaper).
    let deadline = Instant::now() + Duration::from_secs(5);
    while group_alive(pgid) {
        assert!(Instant::now() < deadline, "process group {pgid} is alive");
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        p.order(),
        [
            "_root onentry record",
            "work onentry record",
            "work invoke work"
        ]
    );

    fs::remove_file(p.root().join("wait.flag")).unwrap();
    p.cmd(&["retry", "run-a"]).assert().code(0);
    p.cmd(&["process"]).assert().code(0);
    // Root and state `onentry` run again, then the invoke, then the exits.
    assert_eq!(
        p.order(),
        [
            "_root onentry record",
            "work onentry record",
            "work invoke work",
            "_root onentry record",
            "work onentry record",
            "work invoke work",
            "work onexit record",
            "_root onexit record",
        ]
    );
    assert_eq!(
        transitions(&p.events("run-a")),
        [
            r#"null "claimed" "work" "claim""#,
            r#""work" "retry" "work" "retry""#,
            r#""work" "done" "done" "exit_code""#,
        ]
    );
}

// ---------------------------------------------------------------
// The run lock (docs/reference/messages.md, Run lock)
// ---------------------------------------------------------------

#[test]
fn two_processes_at_once_the_second_skips_the_active_run() {
    let p = Project::new(&[("slow", SLOW)], &[("record", RECORD), ("work", WORK)]);
    p.queue("a.md", "---\nid: run-a\nmachine: slow\n---\n");
    let (mut first, _) = p.working("run-a");
    let before = p.events("run-a");

    p.cmd(&["process"]).assert().code(0);
    assert_eq!(
        p.events("run-a"),
        before,
        "the second process left it alone"
    );
    assert_eq!(
        p.order(),
        [
            "_root onentry record",
            "work onentry record",
            "work invoke work"
        ]
    );

    fs::remove_file(p.root().join("wait.flag")).unwrap();
    let status = wait_with_timeout(&mut first, Duration::from_secs(15));
    assert_eq!(status.code(), Some(0));
    assert_eq!(
        transitions(&p.events("run-a")),
        [
            r#"null "claimed" "work" "claim""#,
            r#""work" "done" "done" "exit_code""#
        ]
    );
    assert_eq!(p.order().len(), 5, "{:?}", p.order());
}

// ---------------------------------------------------------------
// Model (docs/reference/runs.md, Model, step 4)
// ---------------------------------------------------------------

/// Run `pick` with the router replying `reply`; returns the `decision` event and the run's
/// last `transition`.
fn pick_with(reply: Value) -> (Project, Value, Value) {
    let p = Project::new(&[("pick", PICK), ("router", ROUTER)], &[("reply", REPLY)]);
    fs::write(p.root().join("reply.json"), reply.to_string()).unwrap();
    p.queue("a.md", "---\nid: run-a\nmachine: pick\n---\nShip it?\n");
    let process = p.cmd(&["process"]).assert();
    let decision = p.of_type("run-a", "decision").pop().unwrap();
    let last = p.of_type("run-a", "transition").pop().unwrap();
    // `process` exits 1 when it stops on a failed run.
    process.code(if last["to"] == "failed" { 1 } else { 0 });
    (p, decision, last)
}

#[test]
fn router_reply_that_is_not_an_option_gives_error_with_router_error() {
    let (p, decision, last) = pick_with(json!({"event": "merge", "confidence": 0.99}));
    assert_eq!(decision["event"], "error");
    let why = decision["router_error"].as_str().unwrap();
    assert!(why.contains("merge"), "{why}");
    assert_eq!(last["event"], "error");
    assert_eq!(last["to"], "failed");
    // Validated once: one router run, and it finished `done`.
    let children: Vec<String> = p.runs().into_iter().filter(|r| r != "run-a").collect();
    assert_eq!(children.len(), 1, "{children:?}");
    assert_eq!(decision["child_run"], json!(children[0]));
    let child = p.of_type(&children[0], "run_finished");
    assert_eq!(child[0]["state"], "done");
}

#[test]
fn min_confidence_above_the_reported_confidence_gives_unsure() {
    let (_, decision, last) = pick_with(json!({"event": "ship", "confidence": 0.79}));
    assert_eq!(decision["event"], "unsure");
    assert_eq!(decision["pick"], "ship");
    assert_eq!(decision["confidence"], json!(0.79));
    assert_eq!(last["to"], "unsure_end");
    assert_eq!(last["source"], "model");

    // At the threshold, the pick stands.
    let (_, decision, last) = pick_with(json!({"event": "ship", "confidence": 0.8}));
    assert_eq!(decision["event"], "ship");
    assert_eq!(last["to"], "shipped");
}

#[test]
fn missing_confidence_with_min_confidence_gives_unsure() {
    let (_, decision, last) = pick_with(json!({"event": "ship"}));
    assert_eq!(decision["event"], "unsure");
    assert_eq!(decision["pick"], "ship");
    assert!(decision.get("confidence").is_none(), "{decision}");
    assert_eq!(last["to"], "unsure_end");
}

// ---------------------------------------------------------------
// max_depth (docs/reference/runs.md, Sub-machines)
// ---------------------------------------------------------------

#[test]
fn machine_invoke_past_max_depth_gives_error_without_a_child() {
    let p = Project::new(&[("nest", NEST), ("leaf", LEAF)], &[("record", RECORD)]);
    p.queue("a.md", "---\nid: run-a\nmachine: nest\ndepth: 10\n---\n");
    p.cmd(&["process"]).assert().code(0);
    let last = p.of_type("run-a", "transition").pop().unwrap();
    assert_eq!(last["from"], "sub");
    assert_eq!(last["event"], "error");
    assert_eq!(last["to"], "errored");
    assert_eq!(last["source"], "machine");
    assert_eq!(last["error"], "max_depth 10 reached");
    assert_eq!(p.runs(), ["run-a"]);
    assert!(p.of_type("run-a", "waiting").is_empty());
    assert!(p.order().is_empty());

    // One level less: the child runs at depth 10.
    let p = Project::new(&[("nest", NEST), ("leaf", LEAF)], &[("record", RECORD)]);
    p.queue("a.md", "---\nid: run-a\nmachine: nest\ndepth: 9\n---\n");
    p.cmd(&["process"]).assert().code(0);
    assert_eq!(
        p.of_type("run-a", "transition").pop().unwrap()["to"],
        "done"
    );
    let child = p.runs().into_iter().find(|r| r != "run-a").unwrap();
    let message = fs::read_to_string(p.run_dir(&child).join("message.md")).unwrap();
    assert!(message.contains("\ndepth: 10\n"), "{message}");
}

#[test]
fn model_past_max_depth_gives_error_without_a_router_run() {
    let p = Project::new(&[("pick", PICK), ("router", ROUTER)], &[("reply", REPLY)]);
    fs::write(p.root().join("reply.json"), r#"{"event":"ship"}"#).unwrap();
    p.queue("a.md", "---\nid: run-a\nmachine: pick\ndepth: 10\n---\n");
    p.cmd(&["process"]).assert().code(1);
    let decision = p.of_type("run-a", "decision").pop().unwrap();
    assert_eq!(decision["event"], "error");
    assert_eq!(decision["router_error"], "max_depth 10 reached");
    assert_eq!(
        p.of_type("run-a", "transition").pop().unwrap()["to"],
        "failed"
    );
    assert_eq!(p.runs(), ["run-a"]);
}

// ---------------------------------------------------------------
// onexit failures (docs/reference/scripts.md, Events from an invoke)
// ---------------------------------------------------------------

#[test]
fn onexit_failure_is_recorded_in_exit_failures_and_the_target_is_unchanged() {
    let p = Project::new(
        &[("exits", EXITS)],
        &[("record", RECORD), ("fail_exit", FAIL_EXIT)],
    );
    p.queue("a.md", "---\nid: run-a\nmachine: exits\n---\n");
    p.cmd(&["process"]).assert().code(0);
    let events = p.events("run-a");
    let left = events
        .iter()
        .find(|e| e["type"] == "transition" && e["from"] == "work")
        .unwrap();
    assert_eq!(left["event"], "done");
    assert_eq!(left["to"], "next");
    assert_eq!(left["source"], "exit_code");
    assert_eq!(left["exit_failures"], json!(["fail_exit"]));
    // The failing script has its `script` event; the next `onexit` script still ran.
    let failed = events.iter().find(|e| e["script"] == "fail_exit").unwrap();
    assert_eq!(failed["exit_code"], 3);
    assert_eq!(
        p.order(),
        [
            "work invoke record",
            "work onexit record",
            "next invoke record"
        ]
    );
    // Only that transition records one.
    let with_failures = events
        .iter()
        .filter(|e| e.get("exit_failures").is_some())
        .count();
    assert_eq!(with_failures, 1);
    assert_eq!(events.last().unwrap()["state"], "done");
}

// ---------------------------------------------------------------
// The log cap (docs/reference/scripts.md, Execution, Logs)
// ---------------------------------------------------------------

const CAP: usize = 2_097_152;

/// Run `loud` printing `lines` lines of 16 bytes; returns the expected output and the log.
fn loud(lines: usize) -> (Vec<u8>, Vec<u8>) {
    let p = Project::new(&[("loud", LOUD)], &[("big", BIG)]);
    p.queue(
        "a.md",
        &format!("---\nid: run-a\nmachine: loud\nparams: {{ lines: {lines} }}\n---\n"),
    );
    p.cmd(&["process"]).assert().code(0);
    let script = &p.of_type("run-a", "script")[0];
    assert_eq!(script["exit_code"], 0);
    let log = fs::read(p.run_dir("run-a").join(script["log"].as_str().unwrap())).unwrap();
    (b"0123456789abcde\n".repeat(lines), log)
}

#[test]
fn log_over_2_mib_keeps_its_last_2_mib_behind_the_marker() {
    let (output, log) = loud(CAP / 16 + 1000);
    let marker = "[log truncated — showing last 2MB of output]\n".as_bytes();
    assert!(log.starts_with(marker), "{:?}", &log[..80]);
    assert_eq!(log.len(), marker.len() + CAP);
    assert!(log[marker.len()..] == output[output.len() - CAP..]);
}

#[test]
fn log_of_exactly_2_mib_is_kept_whole() {
    let (output, log) = loud(CAP / 16);
    assert_eq!(output.len(), CAP);
    assert!(log == output);
}
