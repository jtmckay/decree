//! `decree emit` (docs/reference/cli.md): parent, depth and trigger, `emits`, `max_depth` and
//! `--param` checks. Each test builds its own `.decree/` in a temp directory.

use assert_cmd::cargo::cargo_bin_cmd;
use assert_cmd::Command;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

mod common;
use common::write_script;

/// `work` may emit `other`; `done` may emit nothing.
const FLOW: &str = "\
name: flow
description: Works, emitting a follow-up for other.
initial: work
states:
  work:
    invoke: work
    emits: [other]
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
";

const OTHER: &str = "\
name: other
description: Finishes at once.
data:
  n: { type: int, default: 0 }
  ok: { type: bool, default: false }
  label: { type: string, default: x }
initial: done
states:
  done: { final: true }
  failed: { final: true }
";

/// Emits a follow-up for `other` with the body on stdin.
const WORK: &str = r#"#!/usr/bin/env bash
printf 'Follow-up from %s\n' "$DECREE_MESSAGE_ID" | decree emit --machine other --param n=7
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
        fs::write(decree.join("machines/flow.yml"), FLOW).unwrap();
        fs::write(decree.join("machines/other.yml"), OTHER).unwrap();
        write_script(&decree.join("scripts/work"), WORK);
        Project { tmp }
    }

    fn decree(&self) -> PathBuf {
        self.tmp.path().join(".decree")
    }

    /// A run folder `runs/<id>/` whose message has frontmatter `depth`, if given.
    fn parent(&self, id: &str, depth: Option<u32>) {
        let dir = self.decree().join("runs").join(id);
        fs::create_dir_all(&dir).unwrap();
        let depth = depth.map(|d| format!("depth: {d}\n")).unwrap_or_default();
        let text = format!("---\nid: {id}\nmachine: flow\n{depth}---\nbody\n");
        fs::write(dir.join("message.md"), text).unwrap();
    }

    fn inbox(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(self.decree().join("inbox"))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    /// `decree emit` with no `DECREE_*` or trace variables but those given, so a test suite
    /// run inside a decree run (a gate script) sees the same results.
    fn emit(&self, env: &[(&str, &str)], args: &[&str]) -> Command {
        let mut cmd = cargo_bin_cmd!("decree");
        cmd.current_dir(self.tmp.path()).env("NO_COLOR", "1");
        for var in [
            "DECREE_MESSAGE_ID",
            "DECREE_MACHINE",
            "DECREE_STATE",
            "TRACEPARENT",
            "TRACESTATE",
        ] {
            cmd.env_remove(var);
        }
        cmd.envs(env.iter().copied()).arg("emit").args(args);
        cmd
    }
}

fn stderr(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn emit_from_a_run_sets_parent_depth_and_trigger() {
    let p = Project::new();
    p.parent("p1", Some(2));
    let env = [
        ("DECREE_MESSAGE_ID", "p1"),
        ("DECREE_MACHINE", "flow"),
        ("DECREE_STATE", "work"),
    ];
    let args = [
        "--machine",
        "other",
        "--param",
        "n=3",
        "--param",
        "ok=true",
        "--param",
        "label=12",
    ];
    let output = p
        .emit(&env, &args)
        .write_stdin("Body\r\nline 2\n")
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    let id = String::from_utf8(output.stdout).unwrap().trim().to_string();
    assert_eq!(p.inbox(), [format!("{id}.md")]);
    assert_eq!(
        fs::read_to_string(p.decree().join("inbox").join(format!("{id}.md"))).unwrap(),
        format!(
            "---\nid: {id}\nmachine: other\nparent: p1\ndepth: 3\ntrigger: emit\n\
             params:\n  n: 3\n  ok: true\n  label: '12'\n---\nBody\r\nline 2\n"
        )
    );
}

#[test]
fn emit_without_a_run_has_no_parent_or_depth() {
    let p = Project::new();
    let output = p
        .emit(&[], &["--machine", "other"])
        .write_stdin("x\n")
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    let id = String::from_utf8(output.stdout).unwrap().trim().to_string();
    assert_eq!(
        fs::read_to_string(p.decree().join("inbox").join(format!("{id}.md"))).unwrap(),
        format!("---\nid: {id}\nmachine: other\ntrigger: emit\n---\nx\n")
    );
}

#[test]
fn emit_of_a_machine_not_in_the_states_emits_exits_1() {
    let p = Project::new();
    p.parent("p1", None);
    for (state, target) in [("work", "flow"), ("done", "other"), ("_root", "other")] {
        let env = [
            ("DECREE_MESSAGE_ID", "p1"),
            ("DECREE_MACHINE", "flow"),
            ("DECREE_STATE", state),
        ];
        let output = p
            .emit(&env, &["--machine", target])
            .write_stdin("x\n")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{state} -> {target}");
        assert!(
            stderr(&output).contains("may not emit"),
            "{}",
            stderr(&output)
        );
    }
    assert!(p.inbox().is_empty());
}

#[test]
fn emit_from_a_run_at_max_depth_exits_1() {
    let p = Project::new();
    p.parent("below", Some(9));
    p.parent("at", Some(10));
    let env = |id| {
        [
            ("DECREE_MESSAGE_ID", id),
            ("DECREE_MACHINE", "flow"),
            ("DECREE_STATE", "work"),
        ]
    };
    let output = p
        .emit(&env("at"), &["--machine", "other"])
        .write_stdin("x\n")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("depth 11 exceeds max_depth 10"),
        "{}",
        stderr(&output)
    );
    assert!(p.inbox().is_empty());

    // One below the limit is allowed, and lands at max_depth.
    p.emit(&env("below"), &["--machine", "other"])
        .write_stdin("x\n")
        .assert()
        .success();
    let text = fs::read_to_string(p.decree().join("inbox").join(&p.inbox()[0])).unwrap();
    assert!(text.contains("depth: 10\n"), "{text}");
}

#[test]
fn emit_with_bad_params_or_machine_exits_1() {
    let p = Project::new();
    let cases: [(&[&str], &str); 5] = [
        (
            &["--machine", "other", "--param", "nope=1"],
            "unknown param `nope`",
        ),
        (
            &["--machine", "other", "--param", "n=x"],
            "param `n` must be of type `int`",
        ),
        (
            &["--machine", "other", "--param", "ok=yes"],
            "param `ok` must be of type `bool`",
        ),
        (
            &["--machine", "other", "--param", "n"],
            "is not `name=value`",
        ),
        (&["--machine", "ghost"], "unknown machine `ghost`"),
    ];
    for (args, want) in cases {
        let output = p.emit(&[], args).write_stdin("x\n").output().unwrap();
        assert_eq!(output.status.code(), Some(1), "{args:?}");
        assert!(
            stderr(&output).contains(want),
            "{args:?}: {}",
            stderr(&output)
        );
    }
    assert!(p.inbox().is_empty());
}

/// The events of run `id`.
fn events(p: &Project, id: &str) -> Vec<serde_json::Value> {
    fs::read_to_string(p.decree().join("runs").join(id).join("events.jsonl"))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

/// The emitted message carries the emitting script's `TRACEPARENT`, so its run joins the
/// trace under that script's span (docs/reference/observability.md, Traces).
#[test]
fn emit_from_a_script_queues_a_follow_up_that_process_runs() {
    let p = Project::new();
    fs::write(
        p.decree().join("inbox/a.md"),
        "---\nid: a\nmachine: flow\n---\nstart\n",
    )
    .unwrap();
    let bin = PathBuf::from(env!("CARGO_BIN_EXE_decree"));
    let path = format!(
        "{}:{}",
        bin.parent().unwrap().display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mut cmd = cargo_bin_cmd!("decree");
    cmd.current_dir(p.tmp.path())
        .env("NO_COLOR", "1")
        .env("PATH", path)
        .env_remove("DECREE_MESSAGE_ID")
        .env_remove("DECREE_MACHINE")
        .env_remove("DECREE_STATE")
        .arg("process")
        .assert()
        .success();
    let mut runs: Vec<String> = fs::read_dir(p.decree().join("runs"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|id| id != "a")
        .collect();
    assert_eq!(runs.len(), 1, "{runs:?}");
    let child = runs.pop().unwrap();
    let a = events(&p, "a");
    let trace = a[0]["trace_id"].as_str().unwrap();
    let script = a.iter().find(|e| e["type"] == "script").unwrap();
    let span = script["span_id"].as_str().unwrap();
    let text = fs::read_to_string(p.decree().join("runs").join(&child).join("message.md")).unwrap();
    assert_eq!(
        text,
        format!(
            "---\nid: {child}\nmachine: other\nparent: a\ndepth: 1\ntrigger: emit\n\
             params:\n  n: 7\ntraceparent: 00-{trace}-{span}-01\nstate: done\n---\n\
             Follow-up from a\n"
        )
    );
    let claim = &events(&p, &child)[0];
    assert_eq!(claim["trace_id"], trace);
    assert_eq!(claim["parent_span_id"], span);
}
