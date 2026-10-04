//! Traces through the binary (docs/reference/observability.md, Traces): one W3C Trace Context
//! trace id across a run, its router run and its child machine run, with the documented
//! parent spans; a message's `traceparent` honoured, and an invalid one ignored; the
//! `TRACEPARENT` a script sees; and `traces.jsonl`, whose every line is an OTLP/JSON
//! `ExportTraceServiceRequest` agreeing with `events.jsonl`, including the run span of an
//! interrupted run written on recovery and the linked run span after `decree retry`. Each
//! test builds its own `.decree/` in a temp directory.

use assert_cmd::cargo::cargo_bin_cmd;
use assert_cmd::Command;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

mod common;
use common::write_script;

#[path = "common/traces.rs"]
mod traces;
use traces::{agree_with_events, attr, is_error, json_lines};

/// Work, ask a router, run `leaf` as a child, then check.
const FLOW: &str = "\
name: flow
description: Work, ask a router, run a child machine, check.
initial: work
states:
  work:
    invoke: work
    transitions: { done: decide }
  decide:
    invoke:
      model: { question: Ship or rework? }
    transitions:
      ship: { target: sub, description: Ship it. }
      rework: { target: failed, description: Rework it. }
  sub:
    invoke: { machine: leaf }
    transitions: { done: gate }
  gate:
    invoke:
      check: { visits: work, equals: 1 }
    transitions: { true: done, false: failed }
  done: { final: true }
  failed: { final: true }
";

const LEAF: &str = "\
name: leaf
description: Run one script.
initial: work
states:
  work:
    invoke: work
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
";

const ROUTER: &str = "\
name: router
description: Always ship.
initial: ask
states:
  ask:
    invoke: reply
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
";

/// Records `<run id> <TRACEPARENT> <TRACESTATE or ->` per execution; exits with the
/// contents of `exit.code` if it exists; waits while `wait.flag` exists.
const WORK: &str = r#"#!/usr/bin/env bash
echo "$DECREE_MESSAGE_ID $TRACEPARENT ${TRACESTATE:--}" >> "$DECREE_PROJECT_ROOT/env.log"
if [ -e "$DECREE_PROJECT_ROOT/wait.flag" ]; then
  echo "$$" > "$DECREE_PROJECT_ROOT/work.pid"
  while [ -e "$DECREE_PROJECT_ROOT/wait.flag" ]; do sleep 0.02; done
fi
exit "$(cat "$DECREE_PROJECT_ROOT/exit.code" 2>/dev/null || echo 0)"
"#;

const REPLY: &str = r#"#!/usr/bin/env bash
echo "$DECREE_MESSAGE_ID $TRACEPARENT ${TRACESTATE:--}" >> "$DECREE_PROJECT_ROOT/env.log"
echo '{"event": "ship", "confidence": 0.9}' > "$DECREE_REPLY"
"#;

/// The W3C Trace Context example `traceparent`.
const TRACEPARENT: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";

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
        for (name, text) in [("flow", FLOW), ("leaf", LEAF), ("router", ROUTER)] {
            fs::write(decree.join(format!("machines/{name}.yml")), text).unwrap();
        }
        write_script(&decree.join("scripts/work"), WORK);
        write_script(&decree.join("scripts/reply"), REPLY);
        Project { tmp }
    }

    fn root(&self) -> &Path {
        self.tmp.path()
    }

    fn run_dir(&self, id: &str) -> PathBuf {
        self.root().join(".decree/runs").join(id)
    }

    fn queue(&self, id: &str, frontmatter: &str) {
        let text = format!("---\nid: {id}\n{frontmatter}---\nDo it.\n");
        fs::write(self.root().join(format!(".decree/inbox/{id}.md")), text).unwrap();
    }

    fn events(&self, id: &str) -> Vec<Value> {
        json_lines(&self.run_dir(id).join("events.jsonl"))
    }

    fn cmd(&self, args: &[&str]) -> Command {
        let mut cmd = cargo_bin_cmd!("decree");
        cmd.current_dir(self.root())
            .env("NO_COLOR", "1")
            .env_remove("TRACEPARENT")
            .env_remove("TRACESTATE")
            .args(args);
        cmd
    }

    fn process(&self) {
        self.cmd(&["process"]).assert().success();
    }

    /// Each script execution's `(run id, TRACEPARENT, TRACESTATE or "-")`, in order.
    fn env_log(&self) -> Vec<(String, String, String)> {
        fs::read_to_string(self.root().join("env.log"))
            .unwrap_or_default()
            .lines()
            .map(|l| {
                let f: Vec<&str> = l.split(' ').collect();
                (f[0].to_string(), f[1].to_string(), f[2].to_string())
            })
            .collect()
    }
}

fn of_type<'e>(events: &'e [Value], kind: &str) -> Vec<&'e Value> {
    events.iter().filter(|e| e["type"] == kind).collect()
}

fn str_of<'v>(v: &'v Value, key: &str) -> &'v str {
    v[key].as_str().unwrap_or_else(|| panic!("no {key} in {v}"))
}

/// The run span of a run's spans: the one named `run <machine>`.
fn run_span(spans: &BTreeMap<String, Value>) -> &Value {
    let runs: Vec<&Value> = spans
        .values()
        .filter(|s| s["name"].as_str().unwrap().starts_with("run "))
        .collect();
    assert_eq!(runs.len(), 1, "{runs:?}");
    runs[0]
}

#[test]
fn a_run_its_router_run_and_its_child_run_share_one_trace_with_the_documented_parents() {
    let p = Project::new();
    p.queue("top", "machine: flow\n");
    p.process();

    let top = p.events("top");
    let trace = str_of(&top[0], "trace_id").to_string();
    let decision = of_type(&top, "decision")[0];
    let router_id = str_of(decision, "child_run");
    let received = of_type(&top, "received")[0];
    let leaf_id = str_of(received, "child");
    let (router, leaf) = (p.events(router_id), p.events(leaf_id));

    // One trace id on every event of the three runs.
    for e in top.iter().chain(&router).chain(&leaf) {
        assert_eq!(e["trace_id"], trace, "{e}");
    }
    // Each run's spans agree with its events; every span is in the trace.
    let spans = agree_with_events(&p.run_dir("top"));
    let router_spans = agree_with_events(&p.run_dir(router_id));
    let leaf_spans = agree_with_events(&p.run_dir(leaf_id));

    // The run: a root span, `run flow`, from the claim to `run_finished`.
    let run = run_span(&spans);
    assert_eq!(run["name"], "run flow");
    assert!(run.get("parentSpanId").is_none(), "{run}");
    assert_eq!(run["spanId"], top[0]["span_id"]);
    assert!(top[0].get("parent_span_id").is_none());
    assert_eq!(attr(run, "decree.state"), Some("done"));
    assert!(!is_error(run));

    // Every other span of the run is a child of the run span; names as documented.
    let mut names: Vec<&str> = spans
        .values()
        .filter(|s| s["spanId"] != run["spanId"])
        .map(|s| {
            assert_eq!(s["parentSpanId"], run["spanId"], "{s}");
            assert_eq!(attr(s, "decree.run_id"), Some("top"));
            assert_eq!(attr(s, "decree.machine"), Some("flow"));
            s["name"].as_str().unwrap()
        })
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "decision check gate",
            "decision model decide",
            "script work/work",
            "wait sub"
        ]
    );
    let script = spans
        .values()
        .find(|s| s["name"] == "script work/work")
        .unwrap();
    assert_eq!(attr(script, "process.exit.code"), Some("0"));
    assert_eq!(attr(script, "decree.attempt"), Some("1"));
    let model = &spans[str_of(decision, "span_id")];
    assert_eq!(attr(model, "decree.event"), Some("ship"));

    // The model decision's span is the parent of the router run's run span, through the
    // router run's traceparent.
    let router_message = fs::read_to_string(p.run_dir(router_id).join("message.md")).unwrap();
    let decision_span = str_of(decision, "span_id");
    assert!(
        router_message.contains(&format!("\ntraceparent: 00-{trace}-{decision_span}-01\n")),
        "{router_message}"
    );
    assert_eq!(router[0]["parent_span_id"], decision_span);
    let router_run = run_span(&router_spans);
    assert_eq!(router_run["name"], "run router");
    assert_eq!(router_run["parentSpanId"], decision_span);
    let reply = router_spans
        .values()
        .find(|s| s["name"] == "script ask/reply")
        .unwrap();
    assert_eq!(reply["parentSpanId"], router_run["spanId"]);

    // The `machine` invoke's wait span is the parent of the child run's run span.
    let wait_span = str_of(received, "span_id");
    assert_eq!(leaf[0]["parent_span_id"], wait_span);
    assert_eq!(run_span(&leaf_spans)["parentSpanId"], wait_span);
    assert_eq!(spans[wait_span]["name"], "wait sub");

    // A span for each run, script and decision.
    assert_eq!(spans.len(), 1 + 1 + 2 + 1);
    assert_eq!(router_spans.len(), 2);
    assert_eq!(leaf_spans.len(), 2);
}

#[test]
fn an_inbox_messages_traceparent_is_honoured() {
    let p = Project::new();
    p.queue(
        "up",
        &format!("machine: leaf\ntraceparent: {TRACEPARENT}\ntracestate: congo=t61rcWkgMzE\n"),
    );
    p.process();
    let events = p.events("up");
    for e in &events {
        assert_eq!(e["trace_id"], "4bf92f3577b34da6a3ce929d0e0e4736", "{e}");
    }
    assert_eq!(events[0]["parent_span_id"], "00f067aa0ba902b7");
    let spans = agree_with_events(&p.run_dir("up"));
    assert_eq!(run_span(&spans)["parentSpanId"], "00f067aa0ba902b7");
    // The message's tracestate reaches the script.
    let env = p.env_log();
    assert_eq!(env[0].2, "congo=t61rcWkgMzE");
}

#[test]
fn an_invalid_traceparent_is_ignored_and_the_run_starts_a_new_trace() {
    let invalid = [
        "00-4BF92F3577B34DA6A3CE929D0E0E4736-00F067AA0BA902B7-01",
        "00-00000000000000000000000000000000-00f067aa0ba902b7-01",
        "00-4bf92f3577b34da6a3ce929d0e0e4736-0000000000000000-01",
        "ff-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
        "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7",
        "not a traceparent",
    ];
    let p = Project::new();
    for (i, value) in invalid.iter().enumerate() {
        p.queue(
            &format!("bad{i}"),
            &format!("machine: leaf\ntraceparent: '{value}'\ntracestate: congo=t61rcWkgMzE\n"),
        );
    }
    p.process();
    let mut traces = Vec::new();
    for i in 0..invalid.len() {
        let id = format!("bad{i}");
        let events = p.events(&id);
        let trace = str_of(&events[0], "trace_id").to_string();
        assert!(!trace.contains("4bf92f3577b34da6a3ce929d0e0e4736"), "{id}");
        assert!(events[0].get("parent_span_id").is_none(), "{id}");
        let spans = agree_with_events(&p.run_dir(&id));
        assert!(run_span(&spans).get("parentSpanId").is_none(), "{id}");
        traces.push(trace);
    }
    traces.sort();
    traces.dedup();
    assert_eq!(
        traces.len(),
        invalid.len(),
        "each run gets its own random trace"
    );
    // No traceparent, no tracestate.
    assert!(p.env_log().iter().all(|(_, _, state)| state == "-"));
}

#[test]
fn a_script_sees_traceparent_naming_the_run_trace_and_its_own_span() {
    let p = Project::new();
    p.queue("top", "machine: flow\n");
    // A TRACEPARENT and TRACESTATE in decree's own environment belong to another trace.
    p.cmd(&["process"])
        .env("TRACEPARENT", TRACEPARENT)
        .env("TRACESTATE", "other=1")
        .assert()
        .success();
    let env = p.env_log();
    assert_eq!(env.len(), 3, "{env:?}");
    for (run, traceparent, tracestate) in env {
        let events = p.events(&run);
        let script = of_type(&events, "script")[0];
        let want = format!(
            "00-{}-{}-01",
            str_of(script, "trace_id"),
            str_of(script, "span_id")
        );
        assert_eq!(traceparent, want, "{run}");
        assert_eq!(tracestate, "-", "{run}");
    }
}

#[test]
fn a_failed_script_and_a_failed_run_have_status_error() {
    let p = Project::new();
    fs::write(p.root().join("exit.code"), "3").unwrap();
    p.queue("fails", "machine: leaf\n");
    p.cmd(&["process"]).assert().code(1);
    let spans = agree_with_events(&p.run_dir("fails"));
    let script = spans
        .values()
        .find(|s| s["name"] == "script work/work")
        .unwrap();
    assert!(is_error(script), "{script}");
    assert_eq!(attr(script, "process.exit.code"), Some("3"));
    let run = run_span(&spans);
    assert!(is_error(run), "{run}");
    assert_eq!(attr(run, "decree.state"), Some("failed"));
}

/// Start `decree process` with `wait.flag` set; wait until the `work` script runs and
/// return decree and the script's pid.
fn working(p: &Project) -> (std::process::Child, i32) {
    fs::write(p.root().join("wait.flag"), "").unwrap();
    let decree = std::process::Command::new(env!("CARGO_BIN_EXE_decree"))
        .current_dir(p.root())
        .env("NO_COLOR", "1")
        .arg("process")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let pid = p.root().join("work.pid");
    let deadline = Instant::now() + Duration::from_secs(30);
    while !fs::read_to_string(&pid).is_ok_and(|t| t.ends_with('\n')) {
        assert!(Instant::now() < deadline, "work never started");
        thread::sleep(Duration::from_millis(5));
    }
    let pid = fs::read_to_string(pid).unwrap().trim().parse().unwrap();
    (decree, pid)
}

#[test]
fn a_crashed_runs_span_is_written_on_recovery_with_error_and_retry_links_a_new_one() {
    let p = Project::new();
    p.queue("crash", "machine: leaf\n");
    let (mut decree, script) = working(&p);
    decree.kill().unwrap();
    assert_eq!(decree.wait().unwrap().signal(), Some(libc::SIGKILL));
    // SAFETY: kill(2) on the script's own process group.
    unsafe { libc::kill(-script, libc::SIGKILL) };
    fs::remove_file(p.root().join("wait.flag")).unwrap();
    // Spans still open when the process is killed are not written.
    assert!(json_lines(&p.run_dir("crash").join("traces.jsonl")).is_empty());

    // Recovery writes the run span, ended at the `interrupted` event, with ERROR.
    p.process();
    let events = p.events("crash");
    let interrupted = of_type(&events, "interrupted")[0];
    assert_eq!(interrupted["cause"], "crash");
    let spans = agree_with_events(&p.run_dir("crash"));
    let first = run_span(&spans).clone();
    assert!(is_error(&first), "{first}");
    assert_eq!(first["spanId"], events[0]["span_id"]);

    // `decree retry` starts a new run span in the same trace, linked to the previous one.
    p.cmd(&["retry", "crash"]).assert().success();
    p.process();
    let events = p.events("crash");
    let retry = events.iter().find(|e| e["source"] == "retry").unwrap();
    let spans = agree_with_events(&p.run_dir("crash"));
    let second = &spans[str_of(retry, "span_id")];
    assert_ne!(second["spanId"], first["spanId"]);
    assert_eq!(second["traceId"], first["traceId"]);
    assert_eq!(second["links"][0]["spanId"], first["spanId"]);
    assert_eq!(second["links"][0]["traceId"], first["traceId"]);
    assert!(!is_error(second), "{second}");
}

#[test]
fn a_run_stopped_by_a_signal_writes_its_span_with_error() {
    let p = Project::new();
    p.queue("stopped", "machine: leaf\n");
    let (mut decree, _) = working(&p);
    // SAFETY: sends SIGTERM to the decree process.
    unsafe { libc::kill(decree.id() as i32, libc::SIGTERM) };
    assert_eq!(decree.wait().unwrap().code(), Some(130));
    fs::remove_file(p.root().join("wait.flag")).unwrap();
    let spans = agree_with_events(&p.run_dir("stopped"));
    // The stopped script has no `script` event and no span; the run span is an error.
    assert_eq!(spans.len(), 1);
    let run = run_span(&spans);
    assert!(is_error(run), "{run}");
    assert_eq!(run["status"]["message"], "interrupted (signal)");
}
