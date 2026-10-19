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
  need: { type: string, enum: [plan, fix], default: fix }
  megapixels: { type: number, default: 1.0 }
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
            "DECREE_MAX_DEPTH",
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
        stderr(&output).contains(
            "depth 11 exceeds max_depth 10 (set DECREE_MAX_DEPTH to change it): run at is at depth 10"
        ),
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
    let cases: [(&[&str], &str); 7] = [
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
        (
            &["--machine", "other", "--param", "need=review"],
            "param `need` is `review`, not one of the allowed values: plan, fix",
        ),
        (
            &["--machine", "other", "--param", "megapixels=x"],
            "param `megapixels` must be of type `number`",
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

#[test]
fn emit_takes_an_enum_value_and_a_number() {
    let p = Project::new();
    for megapixels in ["1.5", "2"] {
        let args = [
            "--machine",
            "other",
            "--param",
            "need=plan",
            "--param",
            &format!("megapixels={megapixels}"),
        ];
        let output = p.emit(&[], &args).write_stdin("x\n").output().unwrap();
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    }
    let texts: Vec<String> = p
        .inbox()
        .iter()
        .map(|name| fs::read_to_string(p.decree().join("inbox").join(name)).unwrap())
        .collect();
    assert!(
        texts.iter().any(|t| t.contains("megapixels: 1.5\n")),
        "{texts:?}"
    );
    assert!(
        texts.iter().any(|t| t.contains("megapixels: 2\n")),
        "{texts:?}"
    );
    assert!(
        texts.iter().all(|t| t.contains("need: plan\n")),
        "{texts:?}"
    );
}

/// The run id of the message a test queues, in the form decree makes it.
const A: &str = "20260101T000000.000000Z-00000a";

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
        p.decree().join(format!("inbox/{A}.md")),
        format!("---\nid: {A}\nmachine: flow\n---\nstart\n"),
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
        .filter(|id| id != A)
        .collect();
    assert_eq!(runs.len(), 1, "{runs:?}");
    let child = runs.pop().unwrap();
    let a = events(&p, A);
    let trace = a[0]["trace_id"].as_str().unwrap();
    let script = a.iter().find(|e| e["type"] == "script").unwrap();
    let span = script["span_id"].as_str().unwrap();
    let text = fs::read_to_string(p.decree().join("runs").join(&child).join("message.md")).unwrap();
    assert_eq!(
        text,
        format!(
            "---\nid: {child}\nmachine: other\nparent: {A}\ndepth: 1\ntrigger: emit\n\
             params:\n  n: 7\ntraceparent: 00-{trace}-{span}-01\nstate: done\n---\n\
             Follow-up from {A}\n"
        )
    );
    let claim = &events(&p, &child)[0];
    assert_eq!(claim["trace_id"], trace);
    assert_eq!(claim["parent_span_id"], span);
}

// --- A whole message on stdin (docs/reference/cli.md, decree emit) ---

/// A migration for `m`, as `decree process` leaves it once it has run.
const MIGRATION: &str = "---\nmachine: m\nnote: kept\n---\n# A\n\nDo the work.\n";

/// One step, then `done`: a migration writes its `processed.md` line on entering a final state.
const M: &str = "\
name: m
description: Takes one step.
initial: step
states:
  step:
    invoke: step
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
";

impl Project {
    /// `decree process` with no `DECREE_*` variables.
    fn process(&self) {
        let mut cmd = cargo_bin_cmd!("decree");
        cmd.current_dir(self.tmp.path()).env("NO_COLOR", "1");
        for var in ["DECREE_MESSAGE_ID", "DECREE_MACHINE", "DECREE_STATE"] {
            cmd.env_remove(var);
        }
        cmd.arg("process").assert().success();
    }

    /// The text of `inbox/<id>.md`.
    fn queued(&self, id: &str) -> String {
        fs::read_to_string(self.decree().join("inbox").join(format!("{id}.md"))).unwrap()
    }
}

fn id_of(output: &std::process::Output) -> String {
    String::from_utf8(output.stdout.clone())
        .unwrap()
        .trim()
        .to_string()
}

#[test]
fn emit_of_a_processed_migration_queues_its_work_again() {
    let p = Project::new();
    fs::write(p.decree().join("machines/m.yml"), M).unwrap();
    write_script(&p.decree().join("scripts/step"), "#!/usr/bin/env bash\n");
    let migration = p.decree().join("migrations/01-a.md");
    fs::write(&migration, MIGRATION).unwrap();
    p.process();
    let processed = fs::read_to_string(p.decree().join("processed.md")).unwrap();
    assert_eq!(processed.trim(), "01-a.md");
    let before: Vec<String> = fs::read_dir(p.decree().join("runs"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();

    let output = p
        .emit(&[], &[])
        .write_stdin(fs::read_to_string(&migration).unwrap())
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    let id = id_of(&output);
    p.process();

    assert!(!before.contains(&id));
    let text = fs::read_to_string(p.decree().join("runs").join(&id).join("message.md")).unwrap();
    assert_eq!(
        text,
        format!(
            "---\nid: {id}\nmachine: m\nnote: kept\ntrigger: emit\nstate: done\n---\n\
             # A\n\nDo the work.\n"
        )
    );
    assert_eq!(text.matches("Do the work.").count(), 1);
    assert_eq!(fs::read_to_string(&migration).unwrap(), MIGRATION);
    assert_eq!(
        fs::read_to_string(p.decree().join("processed.md")).unwrap(),
        processed
    );
}

#[test]
fn emit_with_machine_overrides_the_frontmatter() {
    let p = Project::new();
    let output = p
        .emit(&[], &["--machine", "flow"])
        .write_stdin(MIGRATION)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    let id = id_of(&output);
    assert_eq!(
        p.queued(&id),
        format!(
            "---\nid: {id}\nmachine: flow\nnote: kept\ntrigger: emit\n---\n# A\n\nDo the work.\n"
        )
    );
}

/// A saved run's message loses the keys decree sets; `--param` overrides one param and keeps
/// the rest; an emitting run's `parent` and `depth` replace the saved ones.
#[test]
fn emit_of_a_saved_message_replaces_the_keys_decree_sets() {
    let p = Project::new();
    p.parent("p1", Some(1));
    let saved = format!(
        "---\nid: {A}\nmachine: other\nstate: done\ntrigger: migration\nmigration: 01-a.md\n\
         arrived_as: a.md\nparent: old\ndepth: 5\nparams:\n  n: 2\n  label: y\n---\nBody\n"
    );
    let env = [
        ("DECREE_MESSAGE_ID", "p1"),
        ("DECREE_MACHINE", "flow"),
        ("DECREE_STATE", "work"),
    ];
    let output = p
        .emit(&env, &["--param", "n=9"])
        .write_stdin(saved)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    let id = id_of(&output);
    assert_ne!(id, A);
    assert_eq!(
        p.queued(&id),
        format!(
            "---\nid: {id}\nmachine: other\nparent: p1\ndepth: 2\nparams:\n  n: 9\n  label: y\n\
             trigger: emit\n---\nBody\n"
        )
    );
}

#[test]
fn emit_of_frontmatter_naming_no_machine_exits_1_and_queues_nothing() {
    let p = Project::new();
    let output = p
        .emit(&[], &[])
        .write_stdin("---\nnote: x\n---\nbody\n")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("stdin: line 1: no `machine` key"),
        "{}",
        stderr(&output)
    );
    assert!(p.inbox().is_empty());
}

#[test]
fn emit_of_a_body_without_machine_exits_1() {
    let p = Project::new();
    let output = p.emit(&[], &[]).write_stdin("body\n").output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("--machine"), "{}", stderr(&output));
    assert!(p.inbox().is_empty());
}

/// Frontmatter that is not valid is refused with the error `decree check` gives for the
/// same file in `inbox/`, at the same line, and nothing is queued.
#[test]
fn emit_of_invalid_frontmatter_gives_the_errors_check_gives() {
    let cases = [
        "---\nmachine: other\n",
        "---\nmachine: [x\n---\nbody\n",
        "---\nmachine: ghost\n---\nbody\n",
        "---\nmachine: other\nparams:\n  n: x\n---\nbody\n",
        "---\nmachine: other\nparams:\n  nope: 1\n---\nbody\n",
        "---\n- a\n---\nbody\n",
    ];
    for input in cases {
        let p = Project::new();
        let output = p.emit(&[], &[]).write_stdin(input).output().unwrap();
        assert_eq!(output.status.code(), Some(1), "{input:?}");
        assert!(p.inbox().is_empty(), "{input:?}");
        let got = stderr(&output);

        fs::write(p.decree().join("inbox/x.md"), input).unwrap();
        let check = cargo_bin_cmd!("decree")
            .current_dir(p.tmp.path())
            .env("NO_COLOR", "1")
            .arg("check")
            .output()
            .unwrap();
        let report = String::from_utf8_lossy(&check.stdout).into_owned()
            + &String::from_utf8_lossy(&check.stderr);
        let line = report
            .lines()
            .find_map(|l| l.split_once("inbox/x.md: ").map(|(_, rest)| rest))
            .unwrap_or_else(|| panic!("{input:?}: {report}"));
        let want = line.strip_suffix(" (M2)").unwrap_or(line);
        assert!(
            got.contains(&format!("stdin: {want}")),
            "{input:?}: emit said {got}, check said {want}"
        );
    }
}
