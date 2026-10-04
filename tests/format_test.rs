//! `--format json` on every command that reports something (docs/reference/cli.md,
//! Machine-readable output): stdout is one JSON document that validates against
//! `.decree/schema/v1/cli/<command>.schema.json`, and the exit code is the text mode's, on
//! success and on each error. A command that fails before it has anything to report prints
//! nothing on stdout; the error goes to stderr. `check --format json` and `--format sarif`
//! are tested on every validation case in `validation_test.rs`.
//!
//! A command that changes the project runs in text mode on a copy, so both modes start
//! from the same files.

use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

mod common;
use common::write_script;
#[path = "common/schema.rs"]
mod schema;

/// Fails until `ok.flag` exists in the project root.
const FLAKY: &str = "\
name: flaky
description: Work that fails until ok.flag exists.
initial: work
states:
  work:
    invoke: work
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
";

const FLAKY_WORK: &str = "#!/usr/bin/env bash\n[ -e \"$DECREE_PROJECT_ROOT/ok.flag\" ]\n";

/// Asks a person before shipping.
const DEPLOY: &str = "\
name: deploy
description: Ask a person before shipping.
initial: approval
states:
  approval:
    invoke:
      person:
        question: Ship it?
        ask: ask_person
    transitions:
      approve: { target: shipped, description: Ship it. }
      reject: { target: done, description: Do not ship. }
  shipped: { final: true }
  done: { final: true }
  failed: { final: true }
";

/// Runs while `run.flag` exists in the project root.
const SLOW: &str = "\
name: slow
description: Work that runs while run.flag exists.
initial: work
states:
  work:
    invoke: wait_flag
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
";

const WAIT_FLAG: &str =
    "#!/usr/bin/env bash\nwhile [ -e \"$DECREE_PROJECT_ROOT/run.flag\" ]; do sleep 0.02; done\n";

struct Project {
    tmp: TempDir,
}

impl Project {
    /// `decree init --ai claude`, the test machines and scripts, and their graphs and
    /// schemas, so `decree check` passes without a warning.
    fn new() -> Project {
        let p = Project {
            tmp: TempDir::new().unwrap(),
        };
        run(p.root(), &["init", "--ai", "claude"], "");
        let decree = p.root().join(".decree");
        for (name, yml) in [("flaky", FLAKY), ("deploy", DEPLOY), ("slow", SLOW)] {
            fs::write(decree.join(format!("machines/{name}.yml")), yml).unwrap();
        }
        for (name, text) in [
            ("work", FLAKY_WORK),
            ("ask_person", "#!/usr/bin/env bash\nexit 0\n"),
            ("wait_flag", WAIT_FLAG),
        ] {
            write_script(&decree.join("scripts").join(name), text);
        }
        run(p.root(), &["graph"], "");
        run(p.root(), &["schema"], "");
        p
    }

    fn root(&self) -> &Path {
        self.tmp.path()
    }

    /// `decree <args>` in text mode on a copy of the project, then with `--format json`
    /// on the project itself. Asserts the exit codes are equal and stdout is one JSON
    /// document valid against `schema`, or empty when `empty`; returns (code, document).
    fn json(&self, args: &[&str], stdin: &str, schema: &str, empty: bool) -> (i32, Value) {
        let copy = TempDir::new().unwrap();
        let status = std::process::Command::new("cp")
            .arg("-a")
            .arg(format!("{}/.", self.root().display()))
            .arg(copy.path())
            .status()
            .unwrap();
        assert!(status.success());
        let (text_code, _, _) = run(copy.path(), args, stdin);

        let mut with_format = args.to_vec();
        with_format.extend(["--format", "json"]);
        let (code, stdout, stderr) = run(self.root(), &with_format, stdin);
        assert_eq!(code, text_code, "{args:?}: {stderr}");
        if empty {
            assert_eq!(stdout, "", "{args:?}");
            return (code, Value::Null);
        }
        let doc: Value = serde_json::from_str(&stdout)
            .unwrap_or_else(|e| panic!("{args:?}: not one JSON document ({e}): {stdout}"));
        let errors = schema::errors(&schema::validator(schema), &doc);
        assert!(errors.is_empty(), "{args:?}: {errors:?}\n{doc:#}");
        (code, doc)
    }

    fn touch(&self, name: &str) {
        fs::write(self.root().join(name), "").unwrap();
    }
}

/// `decree <args>` in `dir` with `stdin`: (exit code, stdout, stderr).
fn run(dir: &Path, args: &[&str], stdin: &str) -> (i32, String, String) {
    let out = cargo_bin_cmd!("decree")
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .env_remove("DECREE_MACHINE")
        .env_remove("DECREE_STATE")
        .env_remove("DECREE_MESSAGE_ID")
        .args(args)
        .write_stdin(stdin)
        .output()
        .unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

fn strings(v: &Value) -> Vec<&str> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap())
        .collect()
}

#[test]
fn check_graph_and_schema() {
    let p = Project::new();
    let (code, doc) = p.json(&["check"], "", schema::CLI_CHECK_SCHEMA, false);
    assert_eq!(code, 0);
    assert_eq!(
        doc,
        serde_json::json!({ "valid": true, "errors": [], "warnings": [] })
    );

    // A stale graph and an unknown schema file: warnings, removed by graph and schema.
    let decree = p.root().join(".decree");
    fs::write(decree.join("graph/old.md"), "old\n").unwrap();
    fs::write(decree.join("schema/v1/old.schema.json"), "{}\n").unwrap();
    let (code, doc) = p.json(&["check"], "", schema::CLI_CHECK_SCHEMA, false);
    assert_eq!(code, 0);
    assert_eq!(doc["valid"], true);
    assert_eq!(
        doc["warnings"],
        serde_json::json!([
            { "file": "graph/old.md", "message": "no machine draws it; run `decree graph`" },
            { "file": "schema/v1/old.schema.json", "message": "not one decree writes; run `decree schema`" },
        ])
    );

    let (code, doc) = p.json(&["graph"], "", schema::CLI_GRAPH_SCHEMA, false);
    assert_eq!(code, 0);
    assert!(strings(&doc["written"]).contains(&".decree/graph/system.md"));
    assert_eq!(strings(&doc["removed"]), [".decree/graph/old.md"]);

    let (code, doc) = p.json(&["schema"], "", schema::CLI_SCHEMA_SCHEMA, false);
    assert_eq!(code, 0);
    let expected: Vec<String> = schema::ALL
        .iter()
        .map(|(name, _)| format!(".decree/schema/v1/{name}.schema.json"))
        .collect();
    assert_eq!(strings(&doc["written"]), expected);
    assert_eq!(
        strings(&doc["removed"]),
        [".decree/schema/v1/old.schema.json"]
    );

    // Errors: check reports them in the document; graph fails before it writes anything.
    fs::write(decree.join("inbox/a.md"), "---\nmachine: nope\n---\n").unwrap();
    fs::write(decree.join("cron/c.md"), "---\ncron: '0 * * * *'\n---\n").unwrap();
    let (code, doc) = p.json(&["check"], "", schema::CLI_CHECK_SCHEMA, false);
    assert_eq!(code, 1);
    assert_eq!(doc["valid"], false);
    assert_eq!(doc["errors"].as_array().unwrap().len(), 2, "{doc:#}");
    let (code, _) = p.json(&["graph"], "", schema::CLI_GRAPH_SCHEMA, true);
    assert_eq!(code, 1);
}

#[test]
fn emit_event_retry_status_process_and_prune() {
    let p = Project::new();

    // emit: the id and path; an unknown machine prints nothing on stdout.
    let (code, doc) = p.json(
        &["emit", "--machine", "flaky"],
        "",
        schema::CLI_EMIT_SCHEMA,
        false,
    );
    assert_eq!(code, 0);
    let flaky = doc["id"].as_str().unwrap().to_string();
    assert_eq!(doc["path"], format!(".decree/inbox/{flaky}.md"));
    assert!(p.root().join(doc["path"].as_str().unwrap()).is_file());
    let (code, _) = p.json(
        &["emit", "--machine", "nope"],
        "",
        schema::CLI_EMIT_SCHEMA,
        true,
    );
    assert_eq!(code, 1);
    fs::write(
        p.root().join(".decree/migrations/01-first.md"),
        "---\nmachine: flaky\n---\n",
    )
    .unwrap();

    // process --dry-run: what would run, with machines.
    let dry = ["process", "--dry-run"];
    let (code, doc) = p.json(&dry, "", schema::CLI_PROCESS_SCHEMA, false);
    assert_eq!(code, 0);
    assert_eq!(
        doc,
        serde_json::json!({
            "migrations": [{ "file": "01-first.md", "valid": true, "machine": "flaky" }],
            "inbox": [{ "file": format!("{flaky}.md"), "valid": true, "machine": "flaky" }],
        })
    );
    fs::remove_file(p.root().join(".decree/migrations/01-first.md")).unwrap();

    // flaky fails and stops process; then deploy waits for a reply.
    assert_eq!(run(p.root(), &["process"], "").0, 1);
    let (_, doc) = p.json(
        &["emit", "--machine", "deploy"],
        "",
        schema::CLI_EMIT_SCHEMA,
        false,
    );
    let deploy = doc["id"].as_str().unwrap().to_string();
    assert_eq!(run(p.root(), &["process"], "").0, 0);

    let (code, doc) = p.json(&["status"], "", schema::CLI_STATUS_SCHEMA, false);
    assert_eq!(code, 0);
    assert_eq!(
        doc["counts"],
        serde_json::json!({ "total": 2, "active": 0, "waiting": 1, "pending": 0,
                            "interrupted": 0, "finished": 1 })
    );
    let waiting = &doc["runs"]["waiting"][0];
    assert_eq!(waiting["id"], deploy.as_str());
    assert_eq!(waiting["machine"], "deploy");
    assert_eq!(waiting["state"], "approval");
    assert_eq!(waiting["wait_id"], format!("{deploy}.w1"));
    assert_eq!(strings(&waiting["options"]), ["approve", "reject"]);
    assert_eq!(doc["runs"]["finished"]["failed"][0]["id"], flaky.as_str());

    let (code, doc) = p.json(&["status", &flaky], "", schema::CLI_STATUS_SCHEMA, false);
    assert_eq!(code, 0);
    assert_eq!(doc["status"], "finished");
    assert_eq!(doc["state"], "failed");
    let events = doc["events"].as_array().unwrap();
    assert_eq!(events.last().unwrap()["type"], "run_finished");
    // An unknown run: reported on stderr, exit 0, as text.
    let (code, _) = p.json(&["status", "nope"], "", schema::CLI_STATUS_SCHEMA, true);
    assert_eq!(code, 0);

    // event: the reply's id and path; a run that is not waiting prints nothing.
    let (code, doc) = p.json(
        &["event", &deploy, "approve"],
        "",
        schema::CLI_EVENT_SCHEMA,
        false,
    );
    assert_eq!(code, 0);
    let reply = doc["id"].as_str().unwrap().to_string();
    assert_eq!(doc["path"], format!(".decree/inbox/{reply}.md"));
    let (code, _) = p.json(
        &["event", &flaky, "approve"],
        "",
        schema::CLI_EVENT_SCHEMA,
        true,
    );
    assert_eq!(code, 1);
    let (_, doc) = p.json(&dry, "", schema::CLI_PROCESS_SCHEMA, false);
    assert_eq!(
        doc["inbox"],
        serde_json::json!([{ "file": format!("{reply}.md"), "valid": true, "to": deploy }])
    );

    // retry: the run and its state; a waiting run cannot be retried.
    let (code, _) = p.json(&["retry", &deploy], "", schema::CLI_RETRY_SCHEMA, true);
    assert_eq!(code, 1);
    let (code, doc) = p.json(&["retry", &flaky], "", schema::CLI_RETRY_SCHEMA, false);
    assert_eq!(code, 0);
    assert_eq!(doc, serde_json::json!({ "id": flaky, "state": "work" }));

    p.touch("ok.flag");
    assert_eq!(run(p.root(), &["process"], "").0, 0);

    // prune: what it would delete, then what it deleted.
    let prune = ["prune", "--older-than", "0s"];
    let (code, doc) = p.json(
        &[&prune[..], &["--dry-run"]].concat(),
        "",
        schema::CLI_PRUNE_SCHEMA,
        false,
    );
    assert_eq!(code, 0);
    assert_eq!(doc["dry_run"], true);
    let runs = doc["runs"].as_array().unwrap();
    let ids: Vec<&str> = runs.iter().map(|r| r["id"].as_str().unwrap()).collect();
    let mut expected = [flaky.as_str(), deploy.as_str()];
    expected.sort();
    assert_eq!(ids, expected, "in id order");
    let shipped = runs.iter().find(|r| r["id"] == deploy.as_str()).unwrap();
    assert_eq!(shipped["machine"], "deploy");
    assert_eq!(shipped["state"], "shipped");
    assert!(doc["bytes"].as_u64().unwrap() > 0);
    let (code, doc) = p.json(&prune, "", schema::CLI_PRUNE_SCHEMA, false);
    assert_eq!(code, 0);
    assert_eq!(doc["dry_run"], false);
    assert_eq!(doc["runs"].as_array().unwrap().len(), 2);
    assert!(!p.root().join(".decree/runs").join(&flaky).exists());
}

/// An invalid queued message: process --dry-run lists it as invalid, reports its error on
/// stderr and exits 1, as text.
#[test]
fn process_dry_run_with_an_invalid_message() {
    let p = Project::new();
    fs::write(
        p.root().join(".decree/inbox/a.md"),
        "---\nmachine: nope\n---\n",
    )
    .unwrap();
    let (code, doc) = p.json(
        &["process", "--dry-run"],
        "",
        schema::CLI_PROCESS_SCHEMA,
        false,
    );
    assert_eq!(code, 1);
    assert_eq!(
        doc,
        serde_json::json!({ "migrations": [], "inbox": [{ "file": "a.md", "valid": false }] })
    );
}

/// An active run: `status` names the script it is running now.
#[test]
fn status_of_an_active_run_names_its_running_script() {
    let p = Project::new();
    p.touch("run.flag");
    let id = run(p.root(), &["emit", "--machine", "slow"], "")
        .1
        .trim()
        .to_string();
    let mut process = std::process::Command::new(env!("CARGO_BIN_EXE_decree"))
        .current_dir(p.root())
        .arg("process")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let running: PathBuf = p.root().join(".decree/runs").join(&id).join(".running");
    let deadline = Instant::now() + Duration::from_secs(30);
    while !running.exists() {
        assert!(Instant::now() < deadline, "the script never started");
        thread::sleep(Duration::from_millis(5));
    }

    let (code, stdout, _) = run(p.root(), &["status", "--format", "json"], "");
    fs::remove_file(p.root().join("run.flag")).unwrap();
    assert!(process.wait().unwrap().success());
    assert_eq!(code, 0);
    let doc: Value = serde_json::from_str(&stdout).unwrap();
    let errors = schema::errors(&schema::validator(schema::CLI_STATUS_SCHEMA), &doc);
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(doc["counts"]["active"], 1);
    let active = &doc["runs"]["active"][0];
    assert_eq!(active["id"], id.as_str());
    assert_eq!(active["running"]["script"], "wait_flag");
    assert_eq!(active["running"]["phase"], "invoke");
    assert_eq!(active["running"]["state"], "work");
    assert!(active["running"]["log"]
        .as_str()
        .unwrap()
        .starts_with(&format!(".decree/runs/{id}/0001-work-wait_flag")));
}

/// `--format` where it does not apply is a usage error, exit 2: `process` without
/// `--dry-run` and `status --cron` print text only; `daemon`, `tail`, `init` and `help`
/// take no `--format`.
#[test]
fn format_where_it_does_not_apply_exits_2() {
    let p = Project::new();
    for args in [
        &["process", "--format", "json"][..],
        &["status", "--cron", "--format", "json"],
        &["daemon", "--format", "json"],
        &["tail", "--format", "json"],
        &["help", "--format", "json"],
        &["init", "--format", "json"],
        &["graph", "--format", "sarif"],
    ] {
        let (code, stdout, _) = run(p.root(), args, "");
        assert_eq!((code, stdout.as_str()), (2, ""), "{args:?}");
    }
}
