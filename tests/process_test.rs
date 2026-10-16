//! `decree process` (docs/reference/messages.md, docs/reference/cli.md): inbox claim and validation, and the six
//! migration rules. Each test builds its own `.decree/` in a temp directory.

use assert_cmd::cargo::cargo_bin_cmd;
use assert_cmd::Command;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

mod common;
use common::{is_time_id, write_script};

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

/// Logs start and end to `order.log`; fails (until `fixed.flag` exists), or emits a
/// follow-up, when the params say so.
const WORK: &str = r#"#!/usr/bin/env bash
echo "start $DECREE_MESSAGE_ID" >> "$DECREE_PROJECT_ROOT/order.log"
if [ "$DECREE_DATA_FAIL" = true ] && [ ! -e "$DECREE_PROJECT_ROOT/fixed.flag" ]; then exit 1; fi
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

    /// The run of migration `file`.
    fn run_of(&self, file: &str) -> String {
        common::migration_run(&self.decree(), file)
    }

    fn process(&self) -> Command {
        let mut cmd = cargo_bin_cmd!("decree");
        cmd.current_dir(self.tmp.path())
            .env("NO_COLOR", "1")
            .arg("process");
        cmd
    }

    /// The cause of the failure is fixed: `WORK` passes from now on.
    fn fix(&self) {
        fs::write(self.tmp.path().join("fixed.flag"), "").unwrap();
    }
}

/// `(from, to, source)` of every `transition` event.
fn transitions(events: &[Value]) -> Vec<(String, String, String)> {
    let s = |v: &Value| v.as_str().unwrap_or("-").to_string();
    events
        .iter()
        .filter(|e| e["type"] == "transition")
        .map(|e| (s(&e["from"]), s(&e["to"]), s(&e["source"])))
        .collect()
}

fn t(from: &str, to: &str, source: &str) -> (String, String, String) {
    (from.into(), to.into(), source.into())
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
    p.write(
        "inbox/20260101T000000.000000Z-000000.md",
        "---\nid: 20260101T000000.000000Z-000000\nmachine: nope\n---\nbody\n",
    );
    let out = p.process().assert().code(1).get_output().clone();
    assert!(
        stderr(&out).contains("unknown machine `nope`"),
        "{}",
        stderr(&out)
    );

    assert_eq!(p.runs(), ["20260101T000000.000000Z-000000"]);
    assert!(!p
        .decree()
        .join("inbox/20260101T000000.000000Z-000000.md")
        .exists());
    let events = p.events("20260101T000000.000000Z-000000");
    assert_eq!(events.len(), 2, "{events:?}");
    assert_eq!(events[1]["type"], "run_finished");
    assert_eq!(events[1]["state"], "failed");
    let e = &events[0];
    assert_eq!(e["type"], "transition");
    assert_eq!(e["to"], "failed");
    assert_eq!(e["source"], "invalid_message");
    assert_eq!(e["decree_version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(e["file"], "20260101T000000.000000Z-000000.md");
    assert!(e["error"]
        .as_str()
        .unwrap()
        .contains("unknown machine `nope`"));
    assert!(p
        .read("runs/20260101T000000.000000Z-000000/message.md")
        .contains("state: failed"));
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
fn message_md_keeps_unknown_keys() {
    let p = Project::new();
    p.write(
        "inbox/a.md",
        "---\nmachine: flow\nkeep: me\n---\r\nbody\r\n",
    );
    p.process().assert().success();
    let run = &p.runs()[0];
    let events = p.events(run);
    assert_eq!(events[0]["machine"], "flow");
    assert_eq!(last_state(&events), "done");
    // decree added `id`, `arrived_as` and `trigger`, mirrored `state`, and kept the rest.
    let message = p.read(&format!("runs/{run}/message.md"));
    assert_eq!(
        message,
        format!(
            "---\nid: {run}\narrived_as: a.md\nmachine: flow\nkeep: me\ntrigger: inbox\nstate: done\n---\nbody\r\n"
        )
    );
}

#[test]
fn inbox_runs_in_filename_order() {
    let p = Project::new();
    p.write(
        "inbox/20260101T000000.000001Z-000001.md",
        "---\nid: 20260101T000000.000001Z-000001\nmachine: flow\n---\n",
    );
    p.write(
        "inbox/20260101T000000.000000Z-000000.md",
        "---\nid: 20260101T000000.000000Z-000000\nmachine: flow\n---\n",
    );
    p.write("inbox/.hidden.md", "---\nmachine: flow\n---\n");
    p.process().assert().success();
    assert_eq!(
        p.order(),
        [
            "start 20260101T000000.000000Z-000000",
            "end 20260101T000000.000000Z-000000",
            "start 20260101T000000.000001Z-000001",
            "end 20260101T000000.000001Z-000001"
        ]
    );
    assert!(p.decree().join("inbox/.hidden.md").exists());
}

#[test]
fn a_failed_inbox_run_is_reported_and_process_drains_the_rest() {
    let p = Project::new();
    p.write(
        "inbox/20260101T000000.000000Z-000000.md",
        "---\nid: 20260101T000000.000000Z-000000\nmachine: flow\n---\n",
    );
    p.write(
        "inbox/20260101T000000.000001Z-000001.md",
        "---\nid: 20260101T000000.000001Z-000001\nmachine: flow\nparams: { fail: true }\n---\n",
    );
    p.write(
        "inbox/20260101T000000.000002Z-000002.md",
        "---\nid: 20260101T000000.000002Z-000002\nmachine: flow\n---\n",
    );
    let out = p.process().assert().code(1).get_output().clone();
    let err = stderr(&out);
    assert!(err.contains("run 20260101T000000.000001Z-000001 (20260101T000000.000001Z-000001.md) ended in `failed`"), "{err}");
    assert!(
        err.contains("`decree process --retry 20260101T000000.000001Z-000001`"),
        "{err}"
    );
    // After the queue is empty: every run that failed in the pass, with its retry command.
    let summary = err
        .split_once("1 run(s) ended in `failed` in this pass:")
        .unwrap_or_else(|| panic!("{err}"))
        .1;
    assert!(
        summary.contains("run 20260101T000000.000001Z-000001: decree process --retry 20260101T000000.000001Z-000001"),
        "{err}"
    );
    assert!(
        !summary.contains("20260101T000000.000000Z-000000")
            && !summary.contains("20260101T000000.000002Z-000002"),
        "{err}"
    );

    assert_eq!(
        p.runs(),
        [
            "20260101T000000.000000Z-000000",
            "20260101T000000.000001Z-000001",
            "20260101T000000.000002Z-000002"
        ]
    );
    assert_eq!(
        last_state(&p.events("20260101T000000.000000Z-000000")),
        "done"
    );
    assert_eq!(
        last_state(&p.events("20260101T000000.000001Z-000001")),
        "failed"
    );
    assert_eq!(
        last_state(&p.events("20260101T000000.000002Z-000002")),
        "done"
    );
    assert_eq!(
        p.order(),
        [
            "start 20260101T000000.000000Z-000000",
            "end 20260101T000000.000000Z-000000",
            "start 20260101T000000.000001Z-000001",
            "start 20260101T000000.000002Z-000002",
            "end 20260101T000000.000002Z-000002"
        ]
    );
    assert!(p
        .decree()
        .join("inbox")
        .read_dir()
        .unwrap()
        .next()
        .is_none());

    // `--retry <id>` continues the failed inbox run in its folder, then the pass goes on.
    p.fix();
    p.write(
        "inbox/20260101T000000.000003Z-000003.md",
        "---\nid: 20260101T000000.000003Z-000003\nmachine: flow\n---\n",
    );
    p.process()
        .args(["--retry", "20260101T000000.000001Z-000001"])
        .assert()
        .success();
    assert_eq!(
        p.order()[5..],
        [
            "start 20260101T000000.000001Z-000001",
            "end 20260101T000000.000001Z-000001",
            "start 20260101T000000.000003Z-000003",
            "end 20260101T000000.000003Z-000003"
        ]
    );
    assert_eq!(
        transitions(&p.events("20260101T000000.000001Z-000001")),
        [
            t("-", "work", "claim"),
            t("work", "failed", "exit_code"),
            t("failed", "work", "retry"),
            t("work", "done", "exit_code"),
        ]
    );
}

#[test]
fn a_failed_inbox_run_does_not_block_migrations() {
    let p = Project::new();
    p.write(
        "inbox/20260101T000000.000000Z-000000.md",
        "---\nid: 20260101T000000.000000Z-000000\nmachine: flow\nparams: { fail: true }\n---\n",
    );
    p.write("migrations/01-a.md", "---\nmachine: flow\n---\n");
    p.write("migrations/02-b.md", "---\nmachine: flow\n---\n");
    let out = p.process().assert().code(1).get_output().clone();
    let err = stderr(&out);
    assert!(
        err.contains("run 20260101T000000.000000Z-000000: decree process --retry 20260101T000000.000000Z-000000"),
        "{err}"
    );
    let (a, b) = (p.run_of("01-a.md"), p.run_of("02-b.md"));
    assert_eq!(p.runs(), ["20260101T000000.000000Z-000000", &a, &b]);
    assert_eq!(p.read("processed.md"), "01-a.md\n02-b.md\n");
}

// ---------------------------------------------------------------
// Ids on arrival (docs/reference/messages.md, Lifecycle step 1)
// ---------------------------------------------------------------

/// Set the modification time of `.decree/<rel>` to `secs` after the epoch.
fn set_mtime(p: &Project, rel: &str, secs: u64) {
    let time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs);
    fs::File::options()
        .write(true)
        .open(p.decree().join(rel))
        .unwrap()
        .set_modified(time)
        .unwrap();
}

/// AC: an emitted message, then `01-hi.md` written after it: the emitted message runs
/// first, then `01-hi.md` with a time id, its `message.md` recording `01-hi.md`.
#[test]
fn a_file_written_after_an_emitted_message_runs_after_it_with_a_time_id() {
    let p = Project::new();
    let emitted = cargo_bin_cmd!("decree")
        .current_dir(p.tmp.path())
        .env("NO_COLOR", "1")
        .args(["emit", "--machine", "flow"])
        .write_stdin("emitted\n")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let emitted = String::from_utf8(emitted).unwrap().trim().to_string();
    p.write("inbox/01-hi.md", "---\nmachine: flow\n---\nhi\n");
    p.process().assert().success();

    let runs = p.runs();
    assert_eq!(runs.len(), 2, "{runs:?}");
    assert_eq!(runs[0], emitted);
    let hi = &runs[1];
    assert!(is_time_id(hi), "{hi}");
    assert_eq!(
        p.order(),
        [
            format!("start {emitted}"),
            format!("end {emitted}"),
            format!("start {hi}"),
            format!("end {hi}"),
        ]
    );
    assert_eq!(
        p.read(&format!("runs/{hi}/message.md")),
        format!("---\nid: {hi}\narrived_as: 01-hi.md\nmachine: flow\ntrigger: inbox\nstate: done\n---\nhi\n")
    );
    assert_eq!(p.events(hi)[0]["file"], "01-hi.md");
}

/// AC: `b.md` modified before `a.md`, both written while no process ran: `b.md` runs
/// first. Files modified at the same time run in name order.
#[test]
fn files_first_seen_together_run_in_modification_time_then_name_order() {
    let p = Project::new();
    for name in ["a", "b", "c"] {
        p.write(
            &format!("inbox/{name}.md"),
            &format!("---\nmachine: flow\nname: {name}\n---\n"),
        );
    }
    set_mtime(&p, "inbox/a.md", 2_000_000_000);
    set_mtime(&p, "inbox/b.md", 1_000_000_000);
    set_mtime(&p, "inbox/c.md", 2_000_000_000);
    p.process().assert().success();
    let arrived: Vec<String> = p
        .runs()
        .iter()
        .map(|run| {
            let message = p.read(&format!("runs/{run}/message.md"));
            let line = message.lines().find(|l| l.starts_with("arrived_as: "));
            line.unwrap().to_string()
        })
        .collect();
    assert_eq!(
        arrived,
        ["arrived_as: b.md", "arrived_as: a.md", "arrived_as: c.md"]
    );
    let started: Vec<String> = p
        .order()
        .iter()
        .filter_map(|l| l.strip_prefix("start ").map(String::from))
        .collect();
    assert_eq!(started, p.runs(), "runs start in id order");
}

/// AC: a copy of a finished run's `message.md`, with its `id:`, runs as a new run with a
/// new id; the old run is unchanged. Under any name, its own `<id>.md` included.
#[test]
fn a_copy_of_a_finished_runs_message_runs_again_as_a_new_run() {
    let p = Project::new();
    p.write(
        "inbox/task.md",
        "---\nmachine: flow\nnote: kept\n---\n# Task\n",
    );
    p.process().assert().success();
    let old = p.runs()[0].clone();
    let old_dir = p.decree().join("runs").join(&old);
    let snapshot = |dir: &Path| -> Vec<(String, String)> {
        let mut files: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap())
            .filter(|e| e.path().is_file())
            .map(|e| {
                let name = e.file_name().into_string().unwrap();
                (name, fs::read_to_string(e.path()).unwrap())
            })
            .collect();
        files.sort();
        files
    };
    let before = snapshot(&old_dir);
    let copy = p.read(&format!("runs/{old}/message.md"));
    assert!(copy.contains(&format!("id: {old}\n")) && copy.contains("state: done"));
    p.write("inbox/copy.md", &copy);
    p.write(&format!("inbox/{old}.md"), &copy);
    p.process().assert().success();

    assert_eq!(snapshot(&old_dir), before, "the old run is unchanged");
    let runs = p.runs();
    assert_eq!(runs.len(), 3, "{runs:?}");
    for run in runs.iter().filter(|r| **r != old) {
        assert!(is_time_id(run), "{run}");
        let events = p.events(run);
        assert_eq!(last_state(&events), "done", "{events:?}");
        assert_eq!(events[0]["trigger"], "inbox");
        let message = p.read(&format!("runs/{run}/message.md"));
        assert!(
            message.starts_with(&format!("---\nid: {run}\n")),
            "{message}"
        );
        assert!(message.contains("note: kept\n"), "{message}");
        assert!(message.ends_with("state: done\n---\n# Task\n"), "{message}");
    }
    assert!(p
        .decree()
        .join("inbox")
        .read_dir()
        .unwrap()
        .next()
        .is_none());
}

/// AC: `.draft.md` in `inbox/` is neither renamed nor run.
#[test]
fn a_dot_file_in_the_inbox_is_neither_renamed_nor_run() {
    let p = Project::new();
    let text = "---\nmachine: flow\n---\ndraft\n";
    p.write("inbox/.draft.md", text);
    p.process().assert().success();
    assert!(p.runs().is_empty());
    let names: Vec<String> = fs::read_dir(p.decree().join("inbox"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names, [".draft.md"]);
    assert_eq!(p.read("inbox/.draft.md"), text);
}

/// `decree status` lists a file that arrived by its new name once a pass has seen it, here
/// one that excludes its machine, so it stays queued.
#[test]
fn status_lists_an_arrived_file_by_its_new_name() {
    let p = Project::new();
    p.write("inbox/zz.md", "---\nmachine: flow\n---\n");
    p.process()
        .args(["--exclude-machine", "flow"])
        .assert()
        .success();
    assert!(p.runs().is_empty());
    let names: Vec<String> = fs::read_dir(p.decree().join("inbox"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names.len(), 1);
    let name = &names[0];
    assert!(is_time_id(name.strip_suffix(".md").unwrap()), "{name}");
    assert!(p
        .read(&format!("inbox/{name}"))
        .contains("arrived_as: zz.md\n"));
    let status = cargo_bin_cmd!("decree")
        .current_dir(p.tmp.path())
        .env("NO_COLOR", "1")
        .arg("status")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let status = String::from_utf8(status).unwrap();
    assert!(status.contains(&format!("    {name}\n")), "{status}");
    assert!(!status.contains("zz.md"), "{status}");
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
    // The run has a time id, like every other run, and records the file it came from.
    let id = p.run_of("01-a.md");
    assert!(is_time_id(&id), "{id}");
    assert_eq!(p.runs(), [id.as_str()]);
    assert_eq!(
        p.read(&format!("runs/{id}/message.md")),
        format!("---\nmachine: flow\nid: {id}\ntrigger: migration\nmigration: 01-a.md\nstate: done\n---\n# Task\n")
    );
    let events = p.events(&id);
    assert_eq!(events[0]["trigger"], "migration");
    assert_eq!(events[0]["file"], "01-a.md");
    assert_eq!(p.read("processed.md"), "01-a.md\n");
}

#[test]
fn rule2_migration_in_processed_md_is_skipped() {
    let p = Project::new();
    p.write("migrations/01-a.md", "---\nmachine: flow\n---\n");
    p.write("migrations/02-b.md", "---\nmachine: flow\n---\n");
    p.write("processed.md", "01-a.md\n01-a.md\n");
    p.process().assert().success();
    let b = p.run_of("02-b.md");
    assert_eq!(p.runs(), [b.as_str()]);
    assert_eq!(p.order(), [format!("start {b}"), format!("end {b}")]);
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
    let (a, b) = (p.run_of("01-a.md"), p.run_of("02-b.md"));
    assert_eq!(order.len(), 6, "{order:?}");
    assert_eq!(order[..2], [format!("start {a}"), format!("end {a}")]);
    let followup = order[2].strip_prefix("start ").unwrap();
    assert_ne!(followup, b);
    assert_eq!(order[3], format!("end {followup}"));
    assert_eq!(order[4..], [format!("start {b}"), format!("end {b}")]);
    // Run ids sort in the order the runs were made, migrations' included.
    assert_eq!(p.runs(), [a.as_str(), followup, &b]);
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
    assert!(
        err.contains("Fix the cause, then run `decree process --retry`."),
        "{err}"
    );
    let a = p.run_of("01-a.md");
    assert_eq!(p.runs(), [a.as_str()]);
    assert_eq!(p.order(), [format!("start {a}")]);
    assert_eq!(p.read("processed.md"), "");

    // The failed run still blocks the next pass; nothing re-runs.
    let out = p.process().assert().code(1).get_output().clone();
    let err = stderr(&out);
    assert!(
        err.contains("migration 01-a.md is finished in `failed`"),
        "{err}"
    );
    assert!(err.contains("`decree process --retry`"), "{err}");
    assert_eq!(p.runs(), [a.as_str()]);
    assert_eq!(p.order(), [format!("start {a}")]);
}

#[test]
fn rule4_process_retry_continues_the_blocking_migration_then_the_rest() {
    let p = Project::new();
    p.write(
        "migrations/01-a.md",
        "---\nmachine: flow\nparams: { fail: true }\n---\n",
    );
    p.write("migrations/02-b.md", "---\nmachine: flow\n---\n");
    p.process().assert().code(1);
    let a = p.run_of("01-a.md");
    let before = p.events(&a);

    // `--retry` alone continues the blocking migration in its own folder, keeping its
    // events, then runs the later migration in the same invocation.
    p.fix();
    let out = p
        .process()
        .arg("--retry")
        .assert()
        .success()
        .get_output()
        .clone();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(&format!("run {a} (01-a.md) continues in `work`")),
        "{stdout}"
    );
    let b = p.run_of("02-b.md");
    assert_eq!(p.runs(), [a.as_str(), &b]);
    let after = p.events(&a);
    assert_eq!(after[..before.len()], before[..]);
    assert_eq!(
        transitions(&after),
        [
            t("-", "work", "claim"),
            t("work", "failed", "exit_code"),
            t("failed", "work", "retry"),
            t("work", "done", "exit_code"),
        ]
    );
    assert_eq!(
        p.order(),
        [
            format!("start {a}"),
            format!("start {a}"),
            format!("end {a}"),
            format!("start {b}"),
            format!("end {b}"),
        ]
    );
    assert_eq!(p.read("processed.md"), "01-a.md\n02-b.md\n");

    // Nothing blocks now.
    let out = p
        .process()
        .arg("--retry")
        .assert()
        .code(1)
        .get_output()
        .clone();
    let err = stderr(&out);
    assert!(
        err.contains("nothing to retry: no migration is failed or interrupted"),
        "{err}"
    );
}

/// `decree <args>` in the project, with its output.
fn decree(p: &Project, args: &[&str]) -> std::process::Output {
    cargo_bin_cmd!("decree")
        .current_dir(p.tmp.path())
        .env("NO_COLOR", "1")
        .args(args)
        .output()
        .unwrap()
}

/// AC: `01-a.md` failed; `--retry 01-a`, and separately `--retry 01-a.md`, continue its
/// run, and `decree status 01-a` shows it.
#[test]
fn a_migrations_file_name_or_stem_names_its_run() {
    for name in ["01-a", "01-a.md"] {
        let p = Project::new();
        p.write(
            "migrations/01-a.md",
            "---\nmachine: flow\nparams: { fail: true }\n---\n",
        );
        p.process().assert().code(1);
        let a = p.run_of("01-a.md");

        let status = decree(&p, &["status", name]);
        assert!(status.status.success(), "{}", stderr(&status));
        let stdout = String::from_utf8_lossy(&status.stdout);
        assert!(stdout.contains(&format!("Run {a}\n")), "{stdout}");
        assert!(stdout.contains("  migration: 01-a.md\n"), "{stdout}");
        assert!(stdout.contains("finished in `failed`"), "{stdout}");
        let json = decree(&p, &["status", name, "--format", "json"]);
        let json: Value = serde_json::from_slice(&json.stdout).unwrap();
        assert_eq!(json["id"], a.as_str());
        assert_eq!(json["migration"], "01-a.md");

        p.fix();
        let out = p
            .process()
            .args(["--retry", name])
            .assert()
            .success()
            .get_output()
            .clone();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains(&format!("run {a} (01-a.md) continues in `work`")),
            "{stdout}"
        );
        assert_eq!(p.runs(), [a.as_str()], "the same run continued");
        assert_eq!(last_state(&p.events(&a)), "done");
        assert_eq!(p.read("processed.md"), "01-a.md\n");
    }
}

#[test]
fn a_name_that_is_neither_a_run_nor_a_migrations_run_is_not_found() {
    let p = Project::new();
    p.write("migrations/01-a.md", "---\nmachine: flow\n---\n");
    for args in [
        &["process", "--retry", "01-a"][..],
        &["status", "01-a.md"],
        &["status", "nope"],
        &["tail", "01-a"],
    ] {
        let out = decree(&p, args);
        assert_eq!(out.status.code(), Some(1), "{args:?}");
        let err = stderr(&out);
        assert!(err.contains(args[args.len() - 1]), "{args:?}: {err}");
    }
}

/// AC: an emitted message, then a migration, then another emitted message: `decree status
/// --all` lists their runs in that order.
#[test]
fn status_lists_emitted_and_migration_runs_in_the_order_they_ran() {
    let p = Project::new();
    let emit = |body: &str| {
        let out = cargo_bin_cmd!("decree")
            .current_dir(p.tmp.path())
            .args(["emit", "--machine", "flow"])
            .write_stdin(body.to_string())
            .output()
            .unwrap();
        assert!(out.status.success(), "{}", stderr(&out));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    let first = emit("first\n");
    p.process().assert().success();
    p.write("migrations/01-a.md", "---\nmachine: flow\n---\n");
    p.process().assert().success();
    let a = p.run_of("01-a.md");
    let last = emit("last\n");
    p.process().assert().success();
    assert_eq!(p.runs(), [first.as_str(), &a, &last]);

    let out = decree(&p, &["status", "--all"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let at = |s: &str| stdout.find(s).unwrap_or_else(|| panic!("{s}: {stdout}"));
    assert!(at(&first) < at(&a) && at(&a) < at(&last), "{stdout}");
    assert!(stdout.contains(&format!("{a} (01-a.md)  flow")), "{stdout}");
}

/// Make migration `file`'s run look like one from before migration runs had time ids:
/// named by the file's stem, its `message.md` without `migration:`.
fn as_stem_named_run(p: &Project, file: &str) -> String {
    let id = p.run_of(file);
    let stem = file.strip_suffix(".md").unwrap();
    let runs = p.decree().join("runs");
    fs::rename(runs.join(&id), runs.join(stem)).unwrap();
    for name in ["message.md", "events.jsonl"] {
        let path = runs.join(stem).join(name);
        let text = fs::read_to_string(&path)
            .unwrap()
            .replace(&format!("migration: {file}\n"), "")
            .replace(&id, stem);
        fs::write(&path, text).unwrap();
    }
    stem.to_string()
}

/// AC: a migration run folder named by its stem, from before this change: `process` and
/// `status` still recognise the migration as run, and it is not started again.
#[test]
fn a_stem_named_migration_run_is_still_the_migrations_run() {
    let p = Project::new();
    p.write(
        "migrations/01-a.md",
        "---\nmachine: flow\nparams: { fail: true }\n---\n",
    );
    p.write("migrations/02-b.md", "---\nmachine: flow\n---\n");
    p.process().assert().code(1);
    let stem = as_stem_named_run(&p, "01-a.md");
    assert_eq!(p.runs(), ["01-a"]);

    // It still blocks: nothing new starts.
    let out = p.process().assert().code(1).get_output().clone();
    let err = stderr(&out);
    assert!(
        err.contains("migration 01-a.md is finished in `failed`"),
        "{err}"
    );
    assert_eq!(p.runs(), ["01-a"]);
    let out = decree(&p, &["status"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Blocked: migration 01-a.md is finished in `failed`"),
        "{stdout}"
    );
    for name in ["01-a", "01-a.md"] {
        let out = decree(&p, &["status", name]);
        assert!(
            String::from_utf8_lossy(&out.stdout).contains("Run 01-a\n"),
            "{name}"
        );
    }

    // `--retry 01-a.md` continues it in its own folder; the next migration gets a time id.
    p.fix();
    p.process().args(["--retry", "01-a.md"]).assert().success();
    assert_eq!(last_state(&p.events(&stem)), "done");
    let b = p.run_of("02-b.md");
    assert!(is_time_id(&b), "{b}");
    let mut runs = vec!["01-a".to_string(), b];
    runs.sort();
    assert_eq!(p.runs(), runs);
    assert_eq!(p.read("processed.md"), "01-a.md\n02-b.md\n");

    // A finished one, in the ledger, is not started again either.
    p.process().assert().success();
    assert_eq!(p.runs(), runs);
}

/// `migration:` is decree's alone: a copy of a migration's run placed in `inbox/` loses
/// it, and its run is not a run of that migration.
#[test]
fn a_copy_of_a_migrations_run_is_not_a_run_of_the_migration() {
    let p = Project::new();
    p.write(
        "migrations/01-a.md",
        "---\nmachine: flow\nparams: { fail: true }\n---\n",
    );
    p.process().assert().code(1);
    let a = p.run_of("01-a.md");
    p.write("inbox/copy.md", &p.read(&format!("runs/{a}/message.md")));
    // Queued under an id, as `decree emit` would, the key is dropped at claim.
    let queued = "20260101T000000.000000Z-000000";
    let text = p
        .read(&format!("runs/{a}/message.md"))
        .replace(&format!("id: {a}"), &format!("id: {queued}"))
        .replace("trigger: migration", "trigger: emit");
    p.write(&format!("inbox/{queued}.md"), &text);
    p.fix();
    // The migration blocks, but the inbox drains first.
    p.process().assert().code(1);
    assert_eq!(p.runs().len(), 3);
    assert_eq!(p.run_of("01-a.md"), a, "only the migration's own run");
    for id in p.runs().into_iter().filter(|id| *id != a) {
        let message = p.read(&format!("runs/{id}/message.md"));
        assert!(!message.contains("migration:"), "{message}");
        assert_eq!(last_state(&p.events(&id)), "done");
    }
    let out = decree(&p, &["status", "01-a"]);
    assert!(String::from_utf8_lossy(&out.stdout).contains(&format!("Run {a}\n")));
}

/// Two steps; `step` fails in `second` until `fixed.flag` exists, and logs each state.
const TWO: &str = "\
name: two
description: Two steps.
initial: first
states:
  first:
    invoke: step
    transitions: { done: second }
  second:
    invoke: step
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
";

const STEP: &str = r#"#!/usr/bin/env bash
echo "$DECREE_STATE" >> "$DECREE_PROJECT_ROOT/order.log"
[ "$DECREE_STATE" != second ] || [ -e "$DECREE_PROJECT_ROOT/fixed.flag" ]
"#;

#[test]
fn process_retry_state_resumes_at_the_named_state() {
    let p = Project::new();
    p.write("machines/two.yml", TWO);
    write_script(&p.decree().join("scripts/step"), STEP);
    p.write(
        "inbox/20260101T000000.000000Z-000000.md",
        "---\nid: 20260101T000000.000000Z-000000\nmachine: two\n---\n",
    );
    p.process().assert().code(1);
    assert_eq!(p.order(), ["first", "second"]);

    // Without `--state`, a failed run continues in the state it last left (`second`);
    // `--state first` starts that state again instead.
    p.fix();
    p.process()
        .args([
            "--retry",
            "20260101T000000.000000Z-000000",
            "--state",
            "first",
        ])
        .assert()
        .success();
    assert_eq!(p.order(), ["first", "second", "first", "second"]);
    assert_eq!(
        transitions(&p.events("20260101T000000.000000Z-000000"))[3..],
        [
            t("failed", "first", "retry"),
            t("first", "second", "exit_code"),
            t("second", "done", "exit_code"),
        ]
    );
}

#[test]
fn rule5_ledger_is_written_before_the_final_onentry() {
    let p = Project::new();
    p.write("processed.md", "00-old.md\n");
    p.write("migrations/01-a.md", "---\nmachine: flow\n---\n");
    p.process().assert().success();
    let a = p.run_of("01-a.md");
    assert_eq!(
        p.read(&format!("runs/{a}/ledger-at-commit.txt")),
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
    let a = p.run_of("01-a.md");
    assert_eq!(
        p.read(&format!("runs/{a}/ledger-at-commit.txt")),
        "00-old.md\n01-a.md\n"
    );
    assert_eq!(p.read("processed.md"), "00-old.md\n");
    assert_eq!(last_state(&p.events(&a)), "failed");
    assert_eq!(p.runs(), [a.as_str()]);
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
    p.write("inbox/a.md", "---\nmachine: flow\n---\n");
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

/// A migration, an inbox message and a cron file without `machine:`: `decree check` fails
/// M1, M2 and M3 for them, and `decree process --dry-run` runs nothing.
#[test]
fn messages_without_machine_fail_m1_m2_m3() {
    let p = Project::new();
    fs::create_dir_all(p.decree().join("cron")).unwrap();
    p.write("inbox/task.md", "# No machine key\n");
    p.write(
        "migrations/01-first.md",
        "---\nparams: {}\n---\n# No machine key\n",
    );
    p.write(
        "cron/nightly.md",
        "---\ncron: \"0 2 * * *\"\n---\n# No machine key\n",
    );

    let out = cargo_bin_cmd!("decree")
        .current_dir(p.tmp.path())
        .env("NO_COLOR", "1")
        .arg("check")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "migrations/01-first.md: line 1: no `machine` key (M1)\n\
         inbox/task.md: line 1: no `machine` key (M2)\n\
         cron/nightly.md: line 1: no `machine` key (M3)\n"
    );

    let out = p.process().arg("--dry-run").output().unwrap();
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("no `machine` key"),
        "{}",
        stderr(&out)
    );
    assert!(p.decree().join("inbox/task.md").is_file());
    assert!(p.runs().is_empty());
}
