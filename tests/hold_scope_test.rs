//! What a holding file in `.decree/attention/` holds (docs/reference/cli.md, Attention): a
//! run's file with `holds: true` holds the runs that share its machine's slots, its
//! `concurrency_group` or the machine alone; `pause.md`, and a file decree cannot read, hold
//! every run. A held machine's messages stay queued and its waiting runs get no reply, while
//! the other machines go on; a child run of a held machine does not start, and its parent
//! takes `error`. Each test builds its own `.decree/` in a temp directory.

use assert_cmd::Command;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::{Child, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

mod common;
use common::write_script;

/// One step, `step`: with `fail.flag`, it writes the run's `hold` and fails, as `keep` does
/// when a run leaves changes in a shared work tree.
fn one_step(name: &str, group: Option<&str>) -> String {
    let group = group
        .map(|g| format!("concurrency_group: {g}\n"))
        .unwrap_or_default();
    format!(
        "name: {name}\ndescription: One step.\n{group}initial: step\nstates:\n  step:\n    \
         invoke: step\n    transitions: {{ done: done }}\n  done: {{ final: true }}\n  \
         failed: {{ final: true }}\n"
    )
}

const STEP: &str = r#"#!/usr/bin/env bash
echo "$DECREE_MACHINE" >> "$DECREE_PROJECT_ROOT/ran.log"
if [ -e "$DECREE_PROJECT_ROOT/fail.flag" ]; then
  echo "Changes kept in the work tree." > "$DECREE_RUN_DIR/hold"
  exit 1
fi
"#;

/// Runs machine `a` as a child run.
const PARENT: &str = "\
name: parent
description: Run a as a child.
initial: child
states:
  child:
    invoke: { machine: a }
    transitions:
      done: done
      error: after_error
  after_error:
    invoke: note
    transitions: { done: done }
  done:   { final: true }
  failed: { final: true }
";

/// Asks a person; for a waiting run of a held machine.
const ASK: &str = "\
name: ask
description: Ask.
concurrency_group: tree
initial: approval
states:
  approval:
    invoke:
      reply:
        question: Go?
        notify: notify_person
    transitions:
      go:   { target: done, description: Go. }
      stop: { target: done, description: Stop. }
  done:   { final: true }
  failed: { final: true }
";

struct Project {
    tmp: TempDir,
    seq: std::cell::Cell<u32>,
}

impl Project {
    fn new() -> Project {
        let tmp = TempDir::new().unwrap();
        let decree = tmp.path().join(".decree");
        for dir in ["inbox", "runs"] {
            fs::create_dir_all(decree.join(dir)).unwrap();
        }
        let machines = [
            ("a", one_step("a", Some("tree"))),
            ("b", one_step("b", Some("tree"))),
            ("c", one_step("c", None)),
            ("parent", PARENT.to_string()),
            ("ask", ASK.to_string()),
        ];
        for (name, text) in machines {
            common::write_file(&decree.join(format!("machines/{name}/machine.yml")), text);
        }
        for (name, text) in [
            ("step", STEP),
            (
                "note",
                "#!/usr/bin/env bash\necho after_error >> \"$DECREE_PROJECT_ROOT/ran.log\"\n",
            ),
            ("notify_person", "#!/usr/bin/env bash\n"),
        ] {
            write_script(&decree.join("scripts").join(name), text);
        }
        Project {
            tmp,
            seq: std::cell::Cell::new(0),
        }
    }

    fn root(&self) -> PathBuf {
        self.tmp.path().to_path_buf()
    }

    fn decree(&self) -> PathBuf {
        self.root().join(".decree")
    }

    /// Queue a message for `machine`; returns its id.
    fn queue(&self, machine: &str) -> String {
        let n = self.seq.get();
        self.seq.set(n + 1);
        let id = format!("20260101T000000.{n:06}Z-{n:06}");
        common::write_file(
            &self.decree().join(format!("inbox/{id}.md")),
            format!("---\nid: {id}\nmachine: {machine}\n---\nDo it.\n"),
        );
        id
    }

    fn cmd(&self, args: &[&str]) -> Command {
        let mut cmd = common::decree_at(self.root());
        cmd.env("NO_COLOR", "1").args(args);
        cmd
    }

    /// `daemon --once`: (exit code, stderr).
    fn once(&self) -> (i32, String) {
        let out = self.cmd(&["daemon", "--once"]).output().unwrap();
        (
            out.status.code().unwrap(),
            String::from_utf8(out.stderr).unwrap(),
        )
    }

    fn ran(&self) -> String {
        fs::read_to_string(self.root().join("ran.log")).unwrap_or_default()
    }

    fn last(&self, id: &str) -> Value {
        let text =
            fs::read_to_string(self.decree().join("runs").join(id).join("events.jsonl")).unwrap();
        serde_json::from_str(text.lines().last().unwrap()).unwrap()
    }

    fn queued(&self, id: &str) -> bool {
        self.decree().join(format!("inbox/{id}.md")).is_file()
    }

    /// Fail a run of `a` with a `hold`: its file holds group `tree`.
    fn hold_tree(&self) -> String {
        fs::write(self.root().join("fail.flag"), "").unwrap();
        let held = self.queue("a");
        let (code, _) = self.once();
        assert_eq!(code, 1);
        fs::remove_file(self.root().join("fail.flag")).unwrap();
        fs::remove_file(self.root().join("ran.log")).unwrap();
        let file = self.decree().join(format!("attention/{held}.md"));
        assert!(fs::read_to_string(&file)
            .unwrap()
            .contains("\nholds: true\n"));
        held
    }
}

/// A run's hold holds its group (`a` and `b` in `tree`), not a machine outside it (`c`):
/// `c` runs, `a` and `b` stay queued, and `daemon --once` names the file and exits 1.
#[test]
fn a_runs_hold_holds_its_group_and_not_the_other_machines() {
    let p = Project::new();
    let held = p.hold_tree();
    let (a, b, c) = (p.queue("a"), p.queue("b"), p.queue("c"));
    let (code, stderr) = p.once();
    assert_eq!(code, 1, "{stderr}");
    assert_eq!(p.ran(), "c\n");
    assert!(
        p.queued(&a) && p.queued(&b),
        "held machines' messages stay queued"
    );
    assert_eq!(p.last(&c)["type"], "run_finished");
    assert!(
        stderr.contains(&format!(
            "the runs of group `tree` wait while a file in .decree/attention/ holds them; do \
             what it says, or delete it:\n.decree/attention/{held}.md:\n"
        )),
        "{stderr}"
    );
    assert!(
        stderr.contains("Changes kept in the work tree."),
        "{stderr}"
    );

    // `status` says what the file holds.
    let out = p.cmd(&["status"]).output().unwrap();
    let status = String::from_utf8(out.stdout).unwrap();
    assert!(
        status.contains(&format!(
            ".decree/attention/{held}.md: holds group `tree`: run {held} (a) failed in `step`"
        )),
        "{status}"
    );

    // Deleting the file releases the group: the queue goes on.
    fs::remove_file(p.decree().join(format!("attention/{held}.md"))).unwrap();
    let (code, stderr) = p.once();
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(p.ran(), "c\na\nb\n");
}

/// A waiting run of a held group gets no reply until the hold goes; `pause.md` holds every
/// machine, the ones outside the group too.
#[test]
fn a_held_groups_waiting_run_gets_no_reply_and_a_pause_holds_everything() {
    let p = Project::new();
    let ask = p.queue("ask");
    assert_eq!(p.once().0, 0);
    assert_eq!(p.last(&ask)["type"], "waiting");
    let held = p.hold_tree();
    p.cmd(&["reply", &ask, "go"]).assert().success();
    let (code, _) = p.once();
    assert_eq!(code, 1);
    assert_eq!(
        p.last(&ask)["type"],
        "waiting",
        "the reply waits for the hold"
    );
    fs::remove_file(p.decree().join(format!("attention/{held}.md"))).unwrap();
    assert_eq!(p.once().0, 0);
    assert_eq!(p.last(&ask)["type"], "run_finished");

    p.cmd(&["pause"]).assert().success();
    let c = p.queue("c");
    let (code, stderr) = p.once();
    assert_eq!(code, 1, "{stderr}");
    assert!(p.queued(&c), "a pause holds every machine");
    p.cmd(&["resume"]).assert().success();
    assert_eq!(p.once().0, 0);
    assert!(!p.queued(&c));
}

/// A child run of a held machine does not start: its parent, of a machine nobody holds,
/// takes `error`, and no child run folder is made.
#[test]
fn a_child_of_a_held_machine_does_not_start() {
    let p = Project::new();
    let _held = p.hold_tree();
    let runs_before = fs::read_dir(p.decree().join("runs")).unwrap().count();
    let parent = p.queue("parent");
    let (code, stderr) = p.once();
    assert_eq!(code, 1, "{stderr}");
    assert_eq!(p.ran(), "after_error\n");
    assert_eq!(p.last(&parent)["state"], "done");
    let runs_after = fs::read_dir(p.decree().join("runs")).unwrap().count();
    assert_eq!(runs_after, runs_before + 1, "only the parent's run");
    let events =
        fs::read_to_string(p.decree().join("runs").join(&parent).join("events.jsonl")).unwrap();
    assert!(
        events.contains("group `tree` is held by .decree/attention/"),
        "{events}"
    );
}

/// `decree daemon` names a group's hold once, runs the other machines meanwhile, and goes on
/// with the group once the file is deleted, without a restart.
#[test]
fn a_daemon_runs_the_others_and_goes_on_when_the_hold_goes() {
    let p = Project::new();
    let held = p.hold_tree();
    let a = p.queue("a");
    let daemon: Child = common::std_decree_at(p.root())
        .env("NO_COLOR", "1")
        .args(["daemon", "--interval", "1s"])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let c = p.queue("c");
    let deadline = Instant::now() + Duration::from_secs(30);
    while !p.ran().contains("c\n") {
        assert!(Instant::now() < deadline, "c never ran");
        thread::sleep(Duration::from_millis(50));
    }
    assert!(p.queued(&a));
    fs::remove_file(p.decree().join(format!("attention/{held}.md"))).unwrap();
    while p.queued(&a) || !p.ran().contains("a\n") {
        assert!(Instant::now() < deadline, "a never ran");
        thread::sleep(Duration::from_millis(50));
    }
    // SAFETY: sends SIGTERM to the daemon this test spawned.
    unsafe { libc::kill(daemon.id() as i32, libc::SIGTERM) };
    let out = daemon.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert_eq!(
        stderr.matches("the runs of group `tree` wait").count(),
        1,
        "{stderr}"
    );
    assert_eq!(p.last(&c)["type"], "run_finished");
}

/// The `order` plugin waits for the inbox to empty and for no other run to be pending; a
/// message a hold keeps queued does not count, so `daemon --once` ends rather than waiting on
/// `order` for good. `decree status --format json` marks it `held`.
#[test]
fn order_does_not_wait_for_a_held_message() {
    let p = Project::new();
    p.cmd(&["install", "decree_order"]).assert().success();
    let held = p.hold_tree();
    let a = p.queue("a");
    let out = p.cmd(&["status", "--format", "json"]).output().unwrap();
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        doc["queued"]["held"],
        serde_json::json!([format!("{a}.md")])
    );

    let mut once: Child = common::std_decree_at(p.root())
        .env("NO_COLOR", "1")
        .args(["daemon", "--once"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    let status = loop {
        if let Some(status) = once.try_wait().unwrap() {
            break status;
        }
        if Instant::now() > deadline {
            once.kill().unwrap();
            panic!("daemon --once waited on a held message");
        }
        thread::sleep(Duration::from_millis(100));
    };
    assert_eq!(status.code(), Some(1));
    assert!(p.queued(&a));
    assert!(p.decree().join(format!("attention/{held}.md")).is_file());
}
