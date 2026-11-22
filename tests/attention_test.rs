//! `.decree/attention/` (docs/reference/cli.md, Attention): the file decree writes for a run
//! that waits for a person, was interrupted or ends in `done`; `.md` files it cannot read,
//! which hold the queue; and files it ignores. Each test builds its own `.decree/` in a temp
//! directory and drives the `decree` binary. A failed `develop` run that holds the queue is
//! tested in `develop_test.rs`, a migration waiting for an interrupted child in
//! `failure_test.rs`.

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
#[path = "common/schema.rs"]
mod schema;

const HELLO: &str = "\
name: hello
description: One script, then done.
initial: work
states:
  work:
    invoke: work
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
    invoke:
      reply:
        question: Ship it?
        notify: ask_person
    transitions:
      approve: { target: done, description: Ship. }
      reject: { target: done, description: Do not ship. }
  done: { final: true }
  failed: { final: true }
";

/// Sleeps while `sleep.flag` exists in the project root, recording that it started.
const WORK: &str = r#"#!/usr/bin/env bash
if [ -e "$DECREE_PROJECT_ROOT/sleep.flag" ]; then
  touch "$DECREE_RUN_DIR/started"
  sleep 100 &
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
        for dir in ["machines", "scripts", "inbox", "runs"] {
            fs::create_dir_all(decree.join(dir)).unwrap();
        }
        fs::write(decree.join("processed.md"), "").unwrap();
        common::write_file(&decree.join("machines/hello/machine.yml"), HELLO);
        common::write_file(&decree.join("machines/ask/machine.yml"), ASK);
        write_script(&decree.join("scripts/work"), WORK);
        write_script(&decree.join("scripts/ask_person"), "#!/bin/sh\n");
        Project { tmp }
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.tmp.path().join(".decree").join(rel)
    }

    fn write(&self, rel: &str, text: &str) {
        let path = self.path(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn attention(&self, id: &str) -> PathBuf {
        self.path(&format!("attention/{id}.md"))
    }

    fn cmd(&self, args: &[&str]) -> Command {
        let mut cmd = common::decree_at(self.tmp.path());
        cmd.env("NO_COLOR", "1").args(args);
        cmd
    }

    fn output(&self, args: &[&str]) -> (Option<i32>, String, String) {
        let out = self.cmd(args).output().unwrap();
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    fn last_event(&self, id: &str) -> Value {
        let text = fs::read_to_string(self.path(&format!("runs/{id}/events.jsonl"))).unwrap();
        serde_json::from_str(text.lines().last().unwrap()).unwrap()
    }

    fn runs(&self) -> usize {
        fs::read_dir(self.path("runs")).unwrap().count()
    }
}

/// The frontmatter of the attention file at `path`, checked against `attention.schema.json`.
fn frontmatter(path: &PathBuf) -> Value {
    let text = fs::read_to_string(path).unwrap();
    let json = schema::frontmatter_to_json(&text).unwrap();
    let validator = schema::validator(schema::ATTENTION_SCHEMA);
    assert_eq!(
        schema::errors(&validator, &json),
        Vec::<String>::new(),
        "{text}"
    );
    json
}

#[test]
fn a_run_waiting_for_a_person_has_a_file_until_the_reply_is_delivered() {
    let p = Project::new();
    p.write(
        "inbox/20260101T000000.000000Z-000000.md",
        "---\nid: 20260101T000000.000000Z-000000\nmachine: ask\n---\n# Release 1.2\n",
    );
    let (code, stdout, stderr) = p.output(&["daemon", "--once"]);
    assert_eq!(code, Some(0), "{stderr}");
    assert!(
        stdout.contains("Waiting: run 20260101T000000.000000Z-000000"),
        "{stdout}"
    );
    assert!(
        stderr.ends_with("1 run(s) need attention: .decree/attention/\n"),
        "{stderr}"
    );

    let path = p.attention("20260101T000000.000000Z-000000");
    let fm = frontmatter(&path);
    assert_eq!(fm["holds"], false);
    assert_eq!(fm["status"], "waiting");
    assert_eq!(fm["state"], "approval");
    assert_eq!(fm["message"], "inbox");
    assert_eq!(
        fm["since"],
        p.last_event("20260101T000000.000000Z-000000")["ts"]
    );
    let wait_id = p.last_event("20260101T000000.000000Z-000000")["wait_id"]
        .as_str()
        .unwrap()
        .to_string();
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("\n# Release 1.2\n"), "{text}");
    assert!(text.contains("\nShip it?\n"), "{text}");
    assert!(
        text.contains("- `approve`: Ship.\n- `reject`: Do not ship.\n"),
        "{text}"
    );
    let events: Vec<&str> = text
        .lines()
        .filter(|l| l.trim_start().starts_with("decree reply"))
        .collect();
    assert_eq!(
        events,
        [
            format!("    decree reply {wait_id} approve"),
            format!("    decree reply {wait_id} reject"),
        ]
    );
    // It holds nothing: other runs go on.
    p.write(
        "inbox/20260101T000000.000001Z-000001.md",
        "---\nid: 20260101T000000.000001Z-000001\nmachine: hello\n---\n",
    );
    assert_eq!(p.output(&["daemon", "--once"]).0, Some(0));
    assert_eq!(
        p.last_event("20260101T000000.000001Z-000001")["state"],
        "done"
    );
    // `decree status` lists it first.
    let (_, status, _) = p.output(&["status"]);
    assert!(
        status.starts_with(
            "Attention: .decree/attention/: 1 file(s)\n  .decree/attention/20260101T000000.000000Z-000000.md: run 20260101T000000.000000Z-000000 \
             (ask) waiting in `approval` since "
        ),
        "{status}"
    );

    // Once the reply is delivered, the file is gone.
    p.cmd(&["reply", &wait_id, "approve"]).assert().success();
    assert!(path.exists(), "a queued reply is not delivered yet");
    let (code, _, stderr) = p.output(&["daemon", "--once"]);
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(
        p.last_event("20260101T000000.000000Z-000000")["state"],
        "done"
    );
    assert!(!path.exists());
    assert!(!stderr.contains("need attention"), "{stderr}");
}

/// `decree process` in the background, waiting until run `id`'s script sleeps.
fn sleeping(p: &Project, id: &str) -> Child {
    fs::write(p.tmp.path().join("sleep.flag"), "").unwrap();
    let child = common::std_decree_at(p.tmp.path())
        .env("NO_COLOR", "1")
        .args(["daemon", "--once"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let started = p.path(&format!("runs/{id}/started"));
    let deadline = Instant::now() + Duration::from_secs(30);
    while !started.exists() {
        assert!(Instant::now() < deadline, "the script never started");
        thread::sleep(Duration::from_millis(20));
    }
    child
}

#[test]
fn a_run_interrupted_by_sigint_has_a_file_and_status_starts_with_attention() {
    let p = Project::new();
    p.write(
        "inbox/20260101T000000.000000Z-000000.md",
        "---\nid: 20260101T000000.000000Z-000000\nmachine: hello\n---\n",
    );
    let mut decree = sleeping(&p, "20260101T000000.000000Z-000000");
    // SAFETY: sends SIGINT to the decree process.
    unsafe { libc::kill(decree.id() as i32, libc::SIGINT) };
    let deadline = Instant::now() + Duration::from_secs(15);
    while decree.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline, "decree did not exit");
        thread::sleep(Duration::from_millis(20));
    }
    fs::remove_file(p.tmp.path().join("sleep.flag")).unwrap();

    let (code, status, _) = p.output(&["status"]);
    assert_eq!(code, Some(0));
    assert!(status.starts_with("Attention: "), "{status}");
    let fm = frontmatter(&p.attention("20260101T000000.000000Z-000000"));
    assert_eq!(fm["status"], "interrupted");
    assert_eq!(fm["state"], "work");
    assert_eq!(fm["holds"], false);
    let text = fs::read_to_string(p.attention("20260101T000000.000000Z-000000")).unwrap();
    assert!(
        text.contains("    decree retry 20260101T000000.000000Z-000000\n"),
        "{text}"
    );

    // The retry makes it pending and the file goes; `daemon --once` ends it in `done`.
    let (code, _, stderr) = p.output(&["retry", "20260101T000000.000000Z-000000"]);
    assert_eq!(code, Some(0), "{stderr}");
    assert!(!p.attention("20260101T000000.000000Z-000000").exists());
    let (code, _, stderr) = p.output(&["daemon", "--once"]);
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(
        p.last_event("20260101T000000.000000Z-000000")["state"],
        "done"
    );
    assert!(!p.attention("20260101T000000.000000Z-000000").exists());
}

#[test]
fn a_run_killed_and_found_later_has_a_file_naming_the_crash() {
    let p = Project::new();
    p.write(
        "inbox/20260101T000000.000000Z-000000.md",
        "---\nid: 20260101T000000.000000Z-000000\nmachine: hello\n---\n",
    );
    let mut decree = sleeping(&p, "20260101T000000.000000Z-000000");
    decree.kill().unwrap();
    decree.wait().unwrap();
    // The script is left sleeping; the flag going lets nothing restart it.
    fs::remove_file(p.tmp.path().join("sleep.flag")).unwrap();
    assert!(
        !p.attention("20260101T000000.000000Z-000000").exists(),
        "a killed run writes nothing"
    );

    p.output(&["daemon", "--once"]);
    let fm = frontmatter(&p.attention("20260101T000000.000000Z-000000"));
    assert_eq!(fm["status"], "interrupted");
    let text = fs::read_to_string(p.attention("20260101T000000.000000Z-000000")).unwrap();
    assert!(
        text.contains("decree stopped without recording it"),
        "{text}"
    );
}

#[test]
fn md_files_decree_cannot_read_hold_the_queue_until_deleted() {
    let p = Project::new();
    p.write(
        "attention/notes.md",
        "Pause for the release freeze.\nMore.\n",
    );
    p.write(
        "attention/bad.md",
        "---\nkind: run\nholds: maybe\n---\n# Mine\n",
    );
    p.write(
        "inbox/20260101T000000.000000Z-000000.md",
        "---\nid: 20260101T000000.000000Z-000000\nmachine: hello\n---\n",
    );

    let (code, _, stderr) = p.output(&["daemon", "--once"]);
    assert_eq!(code, Some(1), "{stderr}");
    assert_eq!(p.runs(), 0, "nothing started");
    assert!(
        stderr.contains(
            ".decree/attention/bad.md: holds the queue: invalid frontmatter: missing `run`: \
             ---\n"
        ),
        "{stderr}"
    );
    assert!(
        stderr.contains(
            ".decree/attention/notes.md: holds the queue: no frontmatter: \
             Pause for the release freeze."
        ),
        "{stderr}"
    );
    assert!(
        stderr.contains("2 run(s) need attention: .decree/attention/"),
        "{stderr}"
    );

    let (code, json, _) = p.output(&["status", "--format", "json"]);
    assert_eq!(code, Some(0));
    let doc: Value = serde_json::from_str(&json).unwrap();
    let validator = schema::validator(schema::CLI_STATUS_SCHEMA);
    assert_eq!(schema::errors(&validator, &doc), Vec::<String>::new());
    let attention = doc["attention"].as_array().unwrap();
    assert_eq!(attention.len(), 2);
    assert_eq!(attention[0]["path"], ".decree/attention/bad.md");
    assert_eq!(attention[0]["holds"], true);
    assert!(attention[0]["error"]
        .as_str()
        .unwrap()
        .starts_with("invalid frontmatter: "));
    assert_eq!(attention[1]["path"], ".decree/attention/notes.md");
    assert_eq!(attention[1]["error"], "no frontmatter");
    // decree never rewrites or deletes a file it did not write.
    assert_eq!(
        fs::read_to_string(p.path("attention/notes.md")).unwrap(),
        "Pause for the release freeze.\nMore.\n"
    );

    fs::remove_file(p.path("attention/notes.md")).unwrap();
    assert_eq!(
        p.output(&["daemon", "--once"]).0,
        Some(1),
        "bad.md still holds"
    );
    fs::remove_file(p.path("attention/bad.md")).unwrap();
    let (code, _, stderr) = p.output(&["daemon", "--once"]);
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(
        p.last_event("20260101T000000.000000Z-000000")["state"],
        "done"
    );
}

#[test]
fn other_files_and_folders_in_attention_are_ignored() {
    let p = Project::new();
    p.write("attention/.notes.md.swp", "swap\n");
    p.write("attention/old/x.md", "no frontmatter\n");
    p.write(
        "inbox/20260101T000000.000000Z-000000.md",
        "---\nid: 20260101T000000.000000Z-000000\nmachine: hello\n---\n",
    );
    let (code, _, stderr) = p.output(&["daemon", "--once"]);
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(
        p.last_event("20260101T000000.000000Z-000000")["state"],
        "done"
    );
    assert!(!stderr.contains("need attention"), "{stderr}");
    let (_, status, _) = p.output(&["status"]);
    assert!(!status.contains("Attention:"), "{status}");
}

#[test]
fn a_run_that_ends_in_done_has_no_file() {
    let p = Project::new();
    p.write(
        "inbox/20260101T000000.000000Z-000000.md",
        "---\nid: 20260101T000000.000000Z-000000\nmachine: hello\n---\n",
    );
    let (code, _, stderr) = p.output(&["daemon", "--once"]);
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(
        p.last_event("20260101T000000.000000Z-000000")["state"],
        "done"
    );
    assert!(!p.attention("20260101T000000.000000Z-000000").exists());
    assert!(!stderr.contains("need attention"), "{stderr}");
}

/// Runs `decree pause` from inside its run, as a person would while it goes.
const STOPPER: &str = "\
name: stopper
description: Pause the queue, then finish.
initial: work
states:
  work:
    invoke: stop_queue
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
";

#[test]
fn decree_pause_lets_the_run_finish_and_holds_the_queue_until_resume() {
    let p = Project::new();
    p.write("machines/stopper/machine.yml", STOPPER);
    write_script(
        &p.path("scripts/stop_queue"),
        "#!/bin/sh\n\"${DECREE}\" pause -m \"release freeze\"\n",
    );
    p.write(
        "inbox/20260101T000000.000000Z-000000.md",
        "---\nid: 20260101T000000.000000Z-000000\nmachine: stopper\n---\n",
    );
    let (code, _, stderr) = p.output(&["daemon", "--once"]);
    assert_eq!(code, Some(1), "{stderr}");
    assert_eq!(
        p.last_event("20260101T000000.000000Z-000000")["state"],
        "done",
        "the run is not interrupted"
    );
    // A message queued after the pause does not start. (One already queued for another
    // machine would have started beside the stopper, before the pause: machines run at once.)
    p.write(
        "inbox/20260101T000000.000001Z-000001.md",
        "---\nid: 20260101T000000.000001Z-000001\nmachine: hello\n---\n",
    );
    let (code, _, stderr) = p.output(&["daemon", "--once"]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(
        p.path("inbox/20260101T000000.000001Z-000001.md").exists(),
        "no new run starts"
    );
    assert_eq!(p.runs(), 1);
    assert!(
        stderr.contains(
            "holds the queue; do what it says, or delete it:\n\
             .decree/attention/pause.md:\n# decree pause\n"
        ),
        "{stderr}"
    );
    assert!(stderr.contains("\nrelease freeze\n"), "{stderr}");
    let path = p.attention("pause");
    let fm = frontmatter(&path);
    assert_eq!(fm["kind"], "pause");
    assert_eq!(fm["holds"], true);
    let text = fs::read_to_string(&path).unwrap();

    // `status` lists it, first, in text and JSON.
    let (_, status, _) = p.output(&["status"]);
    assert!(
        status.starts_with(
            "Attention: .decree/attention/: 1 file(s)\n  .decree/attention/pause.md: \
             holds the queue: `decree pause` since "
        ),
        "{status}"
    );
    let (_, json, _) = p.output(&["status", "--format", "json"]);
    let doc: Value = serde_json::from_str(&json).unwrap();
    let validator = schema::validator(schema::CLI_STATUS_SCHEMA);
    assert_eq!(schema::errors(&validator, &doc), Vec::<String>::new());
    assert_eq!(doc["attention"][0]["kind"], "pause");
    assert_eq!(doc["attention"][0]["path"], ".decree/attention/pause.md");

    // A new `process` starts nothing; a retried run waits as `pending`, and the file stays.
    let (code, _, stderr) = p.output(&["daemon", "--once"]);
    assert_eq!(code, Some(1), "{stderr}");
    assert_eq!(p.runs(), 1);
    let (code, stdout, stderr) = p.output(&["retry", "20260101T000000.000000Z-000000"]);
    assert_eq!(code, Some(0), "{stderr}");
    assert!(
        stdout.contains("run 20260101T000000.000000Z-000000 continues in `work`"),
        "{stdout}"
    );
    let (code, _, stderr) = p.output(&["daemon", "--once"]);
    assert_eq!(code, Some(1), "{stderr}");
    assert_eq!(fs::read_to_string(&path).unwrap(), text);
    let (_, status, _) = p.output(&[
        "status",
        "20260101T000000.000000Z-000000",
        "--format",
        "json",
    ]);
    let doc: Value = serde_json::from_str(&status).unwrap();
    assert_eq!(doc["status"], "pending");

    // `decree resume` deletes it: the next `daemon --once` runs the queue.
    let (code, stdout, _) = p.output(&["resume"]);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "resumed: deleted .decree/attention/pause.md\n");
    assert!(!path.exists());
    let (code, _, stderr) = p.output(&["daemon", "--once"]);
    // 20260101T000000.000000Z-000000 pauses the queue again as it is retried; it ends in `done` first.
    assert_eq!(code, Some(1), "{stderr}");
    assert_eq!(
        p.last_event("20260101T000000.000000Z-000000")["state"],
        "done"
    );
    p.output(&["resume"]);
    let (code, _, stderr) = p.output(&["daemon", "--once"]);
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(
        p.last_event("20260101T000000.000001Z-000001")["state"],
        "done"
    );
    assert!(!p.path("inbox/20260101T000000.000001Z-000001.md").exists());
}

#[test]
fn decree_pause_keeps_the_first_file_and_resume_deletes_it_with_json() {
    let p = Project::new();
    let (code, stdout, stderr) = p.output(&["pause"]);
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(
        stdout,
        "paused: the runs executing now finish their step, and nothing more runs until \
         `decree resume` (.decree/attention/pause.md)\n"
    );
    let path = p.attention("pause");
    let text = fs::read_to_string(&path).unwrap();
    assert!(
        text.contains("`decree pause` was run at ") && text.contains("\n    decree resume\n"),
        "{text}"
    );

    let (code, stdout, _) = p.output(&["pause", "-m", "again"]);
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "already paused since .decree/attention/pause.md was written; `decree resume` to go on\n"
    );
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        text,
        "the first `since` stays"
    );

    let validator = schema::validator(schema::CLI_PAUSE_SCHEMA);
    let (code, json, _) = p.output(&["pause", "--format", "json"]);
    assert_eq!(code, Some(0));
    let doc: Value = serde_json::from_str(&json).unwrap();
    assert_eq!(schema::errors(&validator, &doc), Vec::<String>::new());
    assert_eq!(
        doc,
        serde_json::json!({ "attention": ".decree/attention/pause.md", "paused": true })
    );

    // Another file that holds is named after a resume; resume twice says it is not paused.
    p.write("attention/note.md", "hold everything\n");
    let (code, stdout, _) = p.output(&["resume"]);
    assert_eq!(code, Some(0));
    assert!(
        stdout.starts_with(
            "resumed: deleted .decree/attention/pause.md\nstill held:\n  \
             .decree/attention/note.md: holds the queue: no frontmatter"
        ),
        "{stdout}"
    );
    assert!(!path.exists());
    let (code, json, _) = p.output(&["resume", "--format", "json"]);
    assert_eq!(code, Some(0));
    let doc: Value = serde_json::from_str(&json).unwrap();
    assert_eq!(schema::errors(&validator, &doc), Vec::<String>::new());
    assert_eq!(doc["resumed"], false);
    let (_, stdout, _) = p.output(&["resume"]);
    assert!(stdout.starts_with("not paused: "), "{stdout}");

    // Outside a project it fails, and writes nothing.
    let outside = TempDir::new().unwrap();
    common::decree_at(outside.path())
        .arg("pause")
        .assert()
        .code(1);
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
}
