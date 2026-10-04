//! Interrupts, run status and the run lock (docs/reference/messages.md: Lifecycle step 6, Source of
//! truth, Run status, Stopping, Run lock; docs/reference/runs.md, step 1). Each test builds its own
//! `.decree/` in a temp directory and drives the `decree` binary.

use assert_cmd::cargo::cargo_bin_cmd;
use assert_cmd::Command;
use serde_json::{json, Value};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

const SLOW: &str = "\
name: slow
description: Root and state scripts around an invoke that sleeps while a flag file exists.
onentry: [root_entry]
onexit: [root_exit]
initial: work
states:
  work:
    onentry: [work_entry]
    invoke: work
    onexit: [work_exit]
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
";

const ASK: &str = "\
name: ask
description: Ask a person.
initial: approval
states:
  approval:
    invoke: { choose: person, question: Ship it?, ask: ask_person }
    transitions:
      approve: { target: done, description: Ship. }
      reject: { target: done, description: Do not ship. }
  done: { final: true }
  failed: { final: true }
";

/// Appends its own name to `order.log`.
const RECORD: &str = r#"#!/usr/bin/env bash
echo "$(basename "$0")" >> "$DECREE_PROJECT_ROOT/order.log"
"#;

/// While `sleep.flag` exists in the project root: start `sleep 100`, record its pid and the
/// script's own (the leader of its process group), and wait for it.
const WORK: &str = r#"#!/usr/bin/env bash
echo work >> "$DECREE_PROJECT_ROOT/order.log"
if [ -e "$DECREE_PROJECT_ROOT/sleep.flag" ]; then
  sleep 100 &
  echo "$$" > "$DECREE_RUN_DIR/script.pid"
  echo "$!" > "$DECREE_RUN_DIR/child.pid"
  wait
fi
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
        fs::write(decree.join("machines/slow.yml"), SLOW).unwrap();
        fs::write(decree.join("machines/ask.yml"), ASK).unwrap();
        let scripts = [
            ("root_entry", RECORD),
            ("root_exit", RECORD),
            ("work_entry", RECORD),
            ("work_exit", RECORD),
            ("ask_person", RECORD),
            ("work", WORK),
        ];
        for (name, text) in scripts {
            let path = decree.join("scripts").join(name);
            fs::write(&path, text).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        fs::write(tmp.path().join("sleep.flag"), "").unwrap();
        Project { tmp }
    }

    fn decree(&self) -> PathBuf {
        self.tmp.path().join(".decree")
    }

    fn run_dir(&self, id: &str) -> PathBuf {
        self.decree().join("runs").join(id)
    }

    fn write(&self, rel: &str, text: &str) {
        let path = self.decree().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
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

    fn events(&self, id: &str) -> Vec<Value> {
        fs::read_to_string(self.run_dir(id).join("events.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    /// Append an event with the fields every event carries, as decree writes them.
    fn append(&self, id: &str, machine: &str, trigger: &str, fields: Value) {
        let seq = self.events(id).len() + 1;
        let mut event = json!({
            "v": 1, "seq": seq, "ts": "2026-10-02T00:00:00.000Z",
            "run_id": id, "machine": machine, "trigger": trigger,
        });
        event
            .as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        let path = self.run_dir(id).join("events.jsonl");
        let mut text = fs::read_to_string(&path).unwrap_or_default();
        text.push_str(&format!("{event}\n"));
        fs::write(path, text).unwrap();
    }

    /// The `transition` event `decree retry <id>` appends for an interrupted run (section
    /// 8): back into the state it was in. The command itself is tested in `cli_test.rs`.
    fn retry(&self, id: &str, machine: &str, trigger: &str, state: &str) {
        let fields = json!({
            "type": "transition", "from": state, "event": "retry", "to": state,
            "source": "retry", "exit_code": null,
        });
        self.append(id, machine, trigger, fields);
    }

    fn process(&self) -> Command {
        let mut cmd = cargo_bin_cmd!("decree");
        cmd.current_dir(self.tmp.path())
            .env("NO_COLOR", "1")
            .arg("process");
        cmd
    }

    /// `decree process` in the background.
    fn spawn_process(&self) -> Child {
        std::process::Command::new(env!("CARGO_BIN_EXE_decree"))
            .current_dir(self.tmp.path())
            .env("NO_COLOR", "1")
            .arg("process")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap()
    }

    /// Start `decree process` and wait until run `id`'s invoke sleeps. Returns decree, the
    /// script's pid and the sleeping child's pid.
    fn sleeping(&self, id: &str) -> (Child, i32, i32) {
        let decree = self.spawn_process();
        let child_pid = self.run_dir(id).join("child.pid");
        let deadline = Instant::now() + Duration::from_secs(30);
        while !fs::read_to_string(&child_pid).is_ok_and(|t| t.ends_with('\n')) {
            assert!(Instant::now() < deadline, "the invoke never started");
            thread::sleep(Duration::from_millis(20));
        }
        let pid = |name: &str| -> i32 {
            fs::read_to_string(self.run_dir(id).join(name))
                .unwrap()
                .trim()
                .parse()
                .unwrap()
        };
        (decree, pid("script.pid"), pid("child.pid"))
    }

    /// Kill decree with SIGKILL during run `id`'s sleeping invoke, as a crash would, then
    /// clean up the script it leaves behind.
    fn crash_during_sleep(&self, id: &str) {
        let (mut decree, script, _) = self.sleeping(id);
        decree.kill().unwrap();
        assert_eq!(decree.wait().unwrap().signal(), Some(libc::SIGKILL));
        // SAFETY: kill(2) on the script's own process group.
        unsafe { libc::kill(-script, libc::SIGKILL) };
        fs::remove_file(self.tmp.path().join("sleep.flag")).unwrap();
    }
}

fn alive(pid: i32) -> bool {
    // SAFETY: signal 0 only checks that the process exists.
    unsafe { libc::kill(pid, 0) == 0 }
}

fn wait_with_timeout(child: &mut Child, timeout: Duration) -> std::process::ExitStatus {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        assert!(Instant::now() < deadline, "decree did not exit in time");
        thread::sleep(Duration::from_millis(20));
    }
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

fn exists(path: &Path) -> bool {
    path.exists()
}

#[test]
fn sigterm_stops_the_child_appends_interrupted_and_exits_130() {
    let p = Project::new();
    p.write("inbox/a.md", "---\nid: run-a\nmachine: slow\n---\n");
    let (mut decree, script, child) = p.sleeping("run-a");
    // The lock holds decree's pid while the run is stepped.
    assert_eq!(p.read("runs/run-a/.lock"), decree.id().to_string());

    let sent = Instant::now();
    // SAFETY: sends SIGTERM to the decree process.
    unsafe { libc::kill(decree.id() as i32, libc::SIGTERM) };
    let status = wait_with_timeout(&mut decree, Duration::from_secs(15));
    assert_eq!(status.code(), Some(130));
    while alive(child) || alive(script) {
        assert!(
            sent.elapsed() < Duration::from_secs(10),
            "the child is not gone"
        );
        thread::sleep(Duration::from_millis(20));
    }

    let events = p.events("run-a");
    let last = events.last().unwrap();
    assert_eq!(last["type"], "interrupted");
    assert_eq!(last["cause"], "signal");
    assert_eq!(last["state"], "work");
    assert_eq!(last["script"], "work");
    // No `onexit` script ran, and the lock and `.running` are gone.
    assert_eq!(p.order(), ["root_entry", "work_entry", "work"]);
    assert!(!exists(&p.run_dir("run-a").join(".lock")));
    assert!(!exists(&p.run_dir("run-a").join(".running")));

    // A later pass leaves the interrupted run alone.
    p.process().assert().success();
    assert_eq!(p.events("run-a").len(), events.len());
    assert_eq!(p.order(), ["root_entry", "work_entry", "work"]);
}

#[test]
fn sigkill_then_process_marks_the_run_crashed_and_retry_continues_it() {
    let p = Project::new();
    p.write("inbox/a.md", "---\nid: run-a\nmachine: slow\n---\n");
    p.crash_during_sleep("run-a");
    let before = p.events("run-a").len();
    // A stale lock and `.running` are left behind.
    assert!(exists(&p.run_dir("run-a").join(".lock")));
    assert!(p.read("runs/run-a/.running").contains(r#""script":"work""#));

    let out = p.process().output().unwrap();
    assert!(
        stderr(&out).contains("decree retry run-a"),
        "{}",
        stderr(&out)
    );
    let events = p.events("run-a");
    assert_eq!(events.len(), before + 1);
    let last = events.last().unwrap();
    assert_eq!(last["type"], "interrupted");
    assert_eq!(last["cause"], "crash");
    assert_eq!(last["state"], "work");
    // The leftover `.running` names the script, and is gone.
    assert_eq!(last["script"], "work");
    assert!(!exists(&p.run_dir("run-a").join(".running")));
    // Not continued: no script ran again.
    assert_eq!(p.order(), ["root_entry", "work_entry", "work"]);

    // Marked once only.
    p.process().output().unwrap();
    assert_eq!(p.events("run-a").len(), before + 1);
    assert_eq!(p.order(), ["root_entry", "work_entry", "work"]);

    // `decree retry run-a`, then `decree process`: root `onentry` and the state's `onentry`
    // run again, then the invoke, and the run finishes.
    p.retry("run-a", "slow", "inbox", "work");
    p.process().assert().success();
    assert_eq!(
        p.order(),
        [
            "root_entry",
            "work_entry",
            "work",
            "root_entry",
            "work_entry",
            "work",
            "work_exit",
            "root_exit"
        ]
    );
    let events = p.events("run-a");
    assert_eq!(last_state(&events), "done");
    assert_eq!(events.last().unwrap()["type"], "run_finished");
    let transitions: Vec<_> = events
        .iter()
        .filter(|e| e["type"] == "transition")
        .map(|e| format!("{} {} {}", e["from"], e["to"], e["source"]))
        .collect();
    assert_eq!(
        transitions,
        [
            r#"null "work" "claim""#,
            r#""work" "work" "retry""#,
            r#""work" "done" "exit_code""#
        ]
    );
    assert!(p.read("runs/run-a/message.md").contains("state: done"));
    assert!(!exists(&p.run_dir("run-a").join(".lock")));
}

#[test]
fn interrupted_migration_blocks_the_next_and_exits_1_naming_retry() {
    let p = Project::new();
    p.write("migrations/01-a.md", "---\nmachine: slow\n---\n");
    p.write("migrations/02-b.md", "---\nmachine: slow\n---\n");
    p.crash_during_sleep("01-a");

    let out = p.process().assert().code(1).get_output().clone();
    let err = stderr(&out);
    assert!(err.contains("migration 01-a.md is interrupted"), "{err}");
    assert!(err.contains("decree retry 01-a"), "{err}");
    assert!(!exists(&p.run_dir("02-b")));
    assert_eq!(p.read("processed.md"), "");
    let events = p.events("01-a");
    assert_eq!(events.last().unwrap()["cause"], "crash");

    // Still blocked on the next pass.
    p.process().assert().code(1);
    assert!(!exists(&p.run_dir("02-b")));
}

#[test]
fn run_with_a_live_lock_is_active_and_skipped() {
    let p = Project::new();
    p.write(
        "runs/run-a/message.md",
        "---\nid: run-a\nmachine: slow\ntrigger: inbox\nstate: work\n---\n",
    );
    let claim = json!({
        "type": "transition", "from": null, "event": "claimed", "to": "work",
        "source": "claim", "exit_code": null, "file": "a.md",
    });
    p.append("run-a", "slow", "inbox", claim);
    let mut holder = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .unwrap();
    p.write("runs/run-a/.lock", &holder.id().to_string());

    // Neither marked as a crash nor continued, even once it is pending.
    p.process().assert().success();
    assert_eq!(p.events("run-a").len(), 1);
    p.retry("run-a", "slow", "inbox", "work");
    p.process().assert().success();
    assert_eq!(p.events("run-a").len(), 2);
    assert!(p.order().is_empty());
    assert_eq!(p.read("runs/run-a/.lock"), holder.id().to_string());

    // Once the holder is gone, the lock is stale and the pending run continues.
    holder.kill().unwrap();
    holder.wait().unwrap();
    fs::remove_file(p.tmp.path().join("sleep.flag")).unwrap();
    p.process().assert().success();
    assert_eq!(last_state(&p.events("run-a")), "done");
}

#[test]
fn waiting_run_without_a_lock_is_waiting_not_a_crash() {
    let p = Project::new();
    p.write("inbox/a.md", "---\nid: run-a\nmachine: ask\n---\n");
    let out = p.process().assert().success().get_output().clone();
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(stdout.contains("Waiting: run run-a"), "{stdout}");
    assert!(!exists(&p.run_dir("run-a").join(".lock")));
    let events = p.events("run-a");
    assert_eq!(events.last().unwrap()["type"], "waiting");

    let out = p.process().assert().success().get_output().clone();
    assert!(!stderr(&out).contains("interrupted"), "{}", stderr(&out));
    assert_eq!(p.events("run-a"), events);
}

#[test]
fn mirror_that_disagrees_with_the_events_is_rewritten() {
    let p = Project::new();
    // A crash between the `transition` event and the mirror write: events say `work`,
    // message.md still says `done`.
    p.write(
        "runs/run-a/message.md",
        "---\nid: run-a\nmachine: slow\ncustom: kept\ntrigger: inbox\nstate: done\n---\n# Body\r\n",
    );
    let claim = json!({
        "type": "transition", "from": null, "event": "claimed", "to": "work",
        "source": "claim", "exit_code": null, "file": "a.md",
    });
    p.append("run-a", "slow", "inbox", claim);
    p.process().output().unwrap();
    assert_eq!(
        p.read("runs/run-a/message.md"),
        "---\nid: run-a\nmachine: slow\ncustom: kept\ntrigger: inbox\nstate: work\n---\n# Body\r\n"
    );
    assert_eq!(p.events("run-a").last().unwrap()["cause"], "crash");
}
