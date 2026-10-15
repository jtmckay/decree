//! `.decree/attention/` (docs/reference/cli.md, Attention): the file decree writes for a run
//! that waits for a person, was interrupted or ends in `done`; `.md` files it cannot read,
//! which hold the queue; and files it ignores. Each test builds its own `.decree/` in a temp
//! directory and drives the `decree` binary. A failed `develop` run that holds the queue is
//! tested in `develop_test.rs`, a migration waiting for an interrupted child in
//! `failure_test.rs`.

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
      person:
        question: Ship it?
        ask: ask_person
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
        for dir in ["machines", "scripts", "migrations", "inbox", "runs"] {
            fs::create_dir_all(decree.join(dir)).unwrap();
        }
        fs::write(decree.join("processed.md"), "").unwrap();
        fs::write(decree.join("machines/hello.yml"), HELLO).unwrap();
        fs::write(decree.join("machines/ask.yml"), ASK).unwrap();
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
        let mut cmd = cargo_bin_cmd!("decree");
        cmd.current_dir(self.tmp.path())
            .env("NO_COLOR", "1")
            .args(args);
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
        "inbox/a.md",
        "---\nid: run-a\nmachine: ask\n---\n# Release 1.2\n",
    );
    let (code, stdout, stderr) = p.output(&["process"]);
    assert_eq!(code, Some(0), "{stderr}");
    assert!(stdout.contains("Waiting: run run-a"), "{stdout}");
    assert!(
        stderr.ends_with("1 run(s) need attention: .decree/attention/\n"),
        "{stderr}"
    );

    let path = p.attention("run-a");
    let fm = frontmatter(&path);
    assert_eq!(fm["holds"], false);
    assert_eq!(fm["status"], "waiting");
    assert_eq!(fm["state"], "approval");
    assert_eq!(fm["message"], "inbox");
    assert_eq!(fm["since"], p.last_event("run-a")["ts"]);
    let wait_id = p.last_event("run-a")["wait_id"]
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
        .filter(|l| l.trim_start().starts_with("decree event"))
        .collect();
    assert_eq!(
        events,
        [
            format!("    decree event {wait_id} approve"),
            format!("    decree event {wait_id} reject"),
        ]
    );
    // It holds nothing: other runs go on.
    p.write("inbox/b.md", "---\nid: run-b\nmachine: hello\n---\n");
    assert_eq!(p.output(&["process"]).0, Some(0));
    assert_eq!(p.last_event("run-b")["state"], "done");
    // `decree status` lists it first.
    let (_, status, _) = p.output(&["status"]);
    assert!(
        status.starts_with(
            "Attention: .decree/attention/: 1 file(s)\n  .decree/attention/run-a.md: run run-a \
             (ask) waiting in `approval` since "
        ),
        "{status}"
    );

    // Once the reply is delivered, the file is gone.
    p.cmd(&["event", &wait_id, "approve"]).assert().success();
    assert!(path.exists(), "a queued reply is not delivered yet");
    let (code, _, stderr) = p.output(&["process"]);
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(p.last_event("run-a")["state"], "done");
    assert!(!path.exists());
    assert!(!stderr.contains("need attention"), "{stderr}");
}

/// `decree process` in the background, waiting until run `id`'s script sleeps.
fn sleeping(p: &Project, id: &str) -> Child {
    fs::write(p.tmp.path().join("sleep.flag"), "").unwrap();
    let child = std::process::Command::new(env!("CARGO_BIN_EXE_decree"))
        .current_dir(p.tmp.path())
        .env("NO_COLOR", "1")
        .arg("process")
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
    p.write("inbox/a.md", "---\nid: run-a\nmachine: hello\n---\n");
    let mut decree = sleeping(&p, "run-a");
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
    let fm = frontmatter(&p.attention("run-a"));
    assert_eq!(fm["status"], "interrupted");
    assert_eq!(fm["state"], "work");
    assert_eq!(fm["holds"], false);
    let text = fs::read_to_string(p.attention("run-a")).unwrap();
    assert!(
        text.contains("    decree process --retry run-a\n"),
        "{text}"
    );

    // The retry continues it; the file goes as it starts, and it ends in `done`.
    let (code, _, stderr) = p.output(&["process", "--retry", "run-a"]);
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(p.last_event("run-a")["state"], "done");
    assert!(!p.attention("run-a").exists());
}

#[test]
fn a_run_killed_and_found_later_has_a_file_naming_the_crash() {
    let p = Project::new();
    p.write("inbox/a.md", "---\nid: run-a\nmachine: hello\n---\n");
    let mut decree = sleeping(&p, "run-a");
    decree.kill().unwrap();
    decree.wait().unwrap();
    // The script is left sleeping; the flag going lets nothing restart it.
    fs::remove_file(p.tmp.path().join("sleep.flag")).unwrap();
    assert!(
        !p.attention("run-a").exists(),
        "a killed run writes nothing"
    );

    p.output(&["process"]);
    let fm = frontmatter(&p.attention("run-a"));
    assert_eq!(fm["status"], "interrupted");
    let text = fs::read_to_string(p.attention("run-a")).unwrap();
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
    p.write("inbox/a.md", "---\nid: run-a\nmachine: hello\n---\n");

    let (code, _, stderr) = p.output(&["process"]);
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
    assert_eq!(p.output(&["process"]).0, Some(1), "bad.md still holds");
    fs::remove_file(p.path("attention/bad.md")).unwrap();
    let (code, _, stderr) = p.output(&["process"]);
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(p.last_event("run-a")["state"], "done");
}

#[test]
fn other_files_and_folders_in_attention_are_ignored() {
    let p = Project::new();
    p.write("attention/.notes.md.swp", "swap\n");
    p.write("attention/old/x.md", "no frontmatter\n");
    p.write("inbox/a.md", "---\nid: run-a\nmachine: hello\n---\n");
    let (code, _, stderr) = p.output(&["process"]);
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(p.last_event("run-a")["state"], "done");
    assert!(!stderr.contains("need attention"), "{stderr}");
    let (_, status, _) = p.output(&["status"]);
    assert!(!status.contains("Attention:"), "{status}");
}

#[test]
fn a_run_that_ends_in_done_has_no_file() {
    let p = Project::new();
    p.write("inbox/a.md", "---\nid: run-a\nmachine: hello\n---\n");
    let (code, _, stderr) = p.output(&["process"]);
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(p.last_event("run-a")["state"], "done");
    assert!(!p.attention("run-a").exists());
    assert!(!stderr.contains("need attention"), "{stderr}");
}

#[test]
fn prune_deletes_the_file_of_the_run_it_deletes() {
    let p = Project::new();
    // An invalid message fails at its claim, with nothing to hold.
    p.write("inbox/a.md", "---\nid: run-a\nmachine: nope\n---\n");
    p.output(&["process"]);
    assert_eq!(frontmatter(&p.attention("run-a"))["status"], "failed");
    p.cmd(&["prune", "--older-than", "0s"]).assert().success();
    assert!(!p.path("runs/run-a").exists());
    assert!(!p.attention("run-a").exists());
}

/// Runs `decree stop` from inside its run, as a person would while it goes.
const STOPPER: &str = "\
name: stopper
description: Stop the queue, then finish.
initial: work
states:
  work:
    invoke: stop_queue
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
";

#[test]
fn decree_stop_lets_the_run_finish_and_holds_the_queue_until_deleted() {
    let p = Project::new();
    p.write("machines/stopper.yml", STOPPER);
    write_script(
        &p.path("scripts/stop_queue"),
        "#!/bin/sh\n\"${DECREE}\" stop -m \"release freeze\"\n",
    );
    p.write("inbox/1-a.md", "---\nid: run-a\nmachine: stopper\n---\n");
    p.write("inbox/2-b.md", "---\nid: run-b\nmachine: hello\n---\n");

    let (code, _, stderr) = p.output(&["process"]);
    assert_eq!(code, Some(1), "{stderr}");
    assert_eq!(
        p.last_event("run-a")["state"],
        "done",
        "the run is not interrupted"
    );
    assert!(p.path("inbox/2-b.md").exists(), "no new run starts");
    assert_eq!(p.runs(), 1);
    assert!(
        stderr.contains(
            "holds the queue; do what it says, or delete it:\n\
             .decree/attention/manual-stop.md:\n# decree stop\n"
        ),
        "{stderr}"
    );
    assert!(stderr.contains("\nrelease freeze\n"), "{stderr}");
    let path = p.attention("manual-stop");
    let fm = frontmatter(&path);
    assert_eq!(fm["kind"], "manual-stop");
    assert_eq!(fm["holds"], true);
    let text = fs::read_to_string(&path).unwrap();

    // `status` lists it, first, in text and JSON.
    let (_, status, _) = p.output(&["status"]);
    assert!(
        status.starts_with(
            "Attention: .decree/attention/: 1 file(s)\n  .decree/attention/manual-stop.md: \
             holds the queue: `decree stop` since "
        ),
        "{status}"
    );
    let (_, json, _) = p.output(&["status", "--format", "json"]);
    let doc: Value = serde_json::from_str(&json).unwrap();
    let validator = schema::validator(schema::CLI_STATUS_SCHEMA);
    assert_eq!(schema::errors(&validator, &doc), Vec::<String>::new());
    assert_eq!(doc["attention"][0]["kind"], "manual-stop");
    assert_eq!(
        doc["attention"][0]["path"],
        ".decree/attention/manual-stop.md"
    );

    // A new `process` starts nothing; a retried run waits as `pending`, and the file stays.
    let (code, _, stderr) = p.output(&["process"]);
    assert_eq!(code, Some(1), "{stderr}");
    assert_eq!(p.runs(), 1);
    let (code, stdout, stderr) = p.output(&["process", "--retry", "run-a"]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(stdout.contains("run run-a continues in `work`"), "{stdout}");
    assert_eq!(fs::read_to_string(&path).unwrap(), text);
    let (_, status, _) = p.output(&["status", "run-a", "--format", "json"]);
    let doc: Value = serde_json::from_str(&status).unwrap();
    assert_eq!(doc["status"], "pending");

    // Deleting it resumes: the next `process` runs the queue.
    fs::remove_file(&path).unwrap();
    let (code, _, stderr) = p.output(&["process"]);
    // run-a stops the queue again as it is retried; it ends in `done` first.
    assert_eq!(code, Some(1), "{stderr}");
    assert_eq!(p.last_event("run-a")["state"], "done");
    fs::remove_file(&path).unwrap();
    let (code, _, stderr) = p.output(&["process"]);
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(p.last_event("run-b")["state"], "done");
    assert!(!p.path("inbox/2-b.md").exists());
}

#[test]
fn decree_stop_keeps_the_first_file_and_prints_json() {
    let p = Project::new();
    let (code, stdout, stderr) = p.output(&["stop"]);
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(
        stdout,
        "stopping after the current run; delete .decree/attention/manual-stop.md to resume\n"
    );
    let path = p.attention("manual-stop");
    let text = fs::read_to_string(&path).unwrap();
    assert!(
        text.contains("`decree stop` was run at ") && text.contains("Delete this file to resume"),
        "{text}"
    );

    let (code, stdout, _) = p.output(&["stop", "-m", "again"]);
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "already stopped: .decree/attention/manual-stop.md exists; delete it to resume\n"
    );
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        text,
        "the first `since` stays"
    );

    let (code, json, _) = p.output(&["stop", "--format", "json"]);
    assert_eq!(code, Some(0));
    let doc: Value = serde_json::from_str(&json).unwrap();
    let validator = schema::validator(schema::CLI_STOP_SCHEMA);
    assert_eq!(schema::errors(&validator, &doc), Vec::<String>::new());
    assert_eq!(doc["attention"], ".decree/attention/manual-stop.md");

    // Outside a project it fails, and writes nothing.
    let outside = TempDir::new().unwrap();
    cargo_bin_cmd!("decree")
        .current_dir(outside.path())
        .arg("stop")
        .assert()
        .code(1);
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
}
