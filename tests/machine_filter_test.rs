//! `process --machine` and `--exclude-machine`, `DECREE_MACHINES` and
//! `DECREE_EXCLUDE_MACHINES` (docs/reference/cli.md, Several processes; docs/decisions.md,
//! D69): several `process` commands on one project, each running its own machines.

use assert_cmd::cargo::cargo_bin_cmd;
use assert_cmd::Command;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

mod common;
use common::write_script;

/// Logs `<machine> <message id>` to `order.log` and the environment to `env-<id>.txt`; fails
/// when the message says so.
const WORK: &str = r#"#!/usr/bin/env bash
echo "$DECREE_MACHINE $DECREE_MESSAGE_ID" >> "$DECREE_PROJECT_ROOT/order.log"
env | sort > "$DECREE_PROJECT_ROOT/env-$DECREE_MESSAGE_ID.txt"
if [ "$DECREE_DATA_FAIL" = true ]; then exit 1; fi
"#;

struct Project {
    tmp: TempDir,
}

impl Project {
    /// Machines `a` and `b`, each one `work` state, and `p`, whose `work` is followed by
    /// `call`, which invokes `b` as a child run.
    fn new() -> Project {
        let tmp = TempDir::new().unwrap();
        let decree = tmp.path().join(".decree");
        for dir in ["machines", "scripts", "migrations", "inbox", "runs"] {
            fs::create_dir_all(decree.join(dir)).unwrap();
        }
        fs::write(decree.join("processed.md"), "").unwrap();
        for (name, after, call) in [
            ("a", "done", ""),
            ("b", "done", ""),
            (
                "p",
                "call",
                "  call:\n    invoke: { machine: b }\n    transitions: { done: done }\n",
            ),
        ] {
            fs::write(
                decree.join(format!("machines/{name}.yml")),
                format!(
                    "name: {name}\ndescription: One script.\n\
                     data:\n  fail: {{ type: bool, default: false }}\n\
                     initial: work\nstates:\n  work:\n    invoke: work\n    \
                     transitions: {{ done: {after} }}\n{call}  \
                     done: {{ final: true }}\n  failed: {{ final: true }}\n"
                ),
            )
            .unwrap();
        }
        write_script(&decree.join("scripts/work"), WORK);
        Project { tmp }
    }

    fn decree(&self) -> PathBuf {
        self.tmp.path().join(".decree")
    }

    fn write(&self, rel: &str, text: &str) {
        fs::write(self.decree().join(rel), text).unwrap();
    }

    /// Queue `inbox/<id>.md` for `machine`, with extra frontmatter lines `extra`.
    fn queue(&self, id: &str, machine: &str, extra: &str) {
        self.write(
            &format!("inbox/{id}.md"),
            &format!("---\nid: {id}\nmachine: {machine}\n{extra}---\nbody\n"),
        );
    }

    fn inbox(&self) -> Vec<String> {
        let mut files: Vec<String> = fs::read_dir(self.decree().join("inbox"))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        files.sort();
        files
    }

    fn runs(&self) -> Vec<String> {
        let mut runs: Vec<String> = fs::read_dir(self.decree().join("runs"))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        runs.sort();
        runs
    }

    fn order(&self) -> Vec<String> {
        fs::read_to_string(self.tmp.path().join("order.log"))
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect()
    }

    fn decree_cmd(&self, args: &[&str]) -> Command {
        let mut cmd = cargo_bin_cmd!("decree");
        cmd.current_dir(self.tmp.path())
            .env("NO_COLOR", "1")
            .env_remove("DECREE_MACHINES")
            .env_remove("DECREE_EXCLUDE_MACHINES")
            .args(args);
        cmd
    }

    fn process(&self, args: &[&str]) -> Command {
        let mut cmd = self.decree_cmd(&["process"]);
        cmd.args(args);
        cmd
    }
}

fn stdout(cmd: &mut Command) -> String {
    String::from_utf8(cmd.assert().success().get_output().stdout.clone()).unwrap()
}

/// AC: `--machine a` runs only `a`'s messages and leaves `b`'s in `inbox/`; then
/// `--exclude-machine a` runs `b`'s.
#[test]
fn machine_runs_only_its_machines_and_exclude_machine_the_rest() {
    let p = Project::new();
    p.queue("20260101T000000.000000Z-000000", "a", "");
    p.queue("20260101T000000.000001Z-000001", "b", "");
    p.queue("20260101T000000.000002Z-000002", "a", "");
    p.process(&["--machine", "a"]).assert().success();
    assert_eq!(
        p.order(),
        [
            "a 20260101T000000.000000Z-000000",
            "a 20260101T000000.000002Z-000002"
        ]
    );
    assert_eq!(p.inbox(), ["20260101T000000.000001Z-000001.md"]);
    assert_eq!(
        p.runs(),
        [
            "20260101T000000.000000Z-000000",
            "20260101T000000.000002Z-000002"
        ]
    );

    p.process(&["--exclude-machine", "a"]).assert().success();
    assert_eq!(
        p.order(),
        [
            "a 20260101T000000.000000Z-000000",
            "a 20260101T000000.000002Z-000002",
            "b 20260101T000000.000001Z-000001"
        ]
    );
    assert!(p.inbox().is_empty());
}

/// The flags repeat and take comma-separated lists.
#[test]
fn flags_repeat_and_take_lists() {
    let p = Project::new();
    p.queue("20260101T000000.000000Z-000000", "a", "");
    p.queue("20260101T000000.000001Z-000001", "b", "");
    p.process(&["--machine", "a,b"]).assert().success();
    assert_eq!(
        p.order(),
        [
            "a 20260101T000000.000000Z-000000",
            "b 20260101T000000.000001Z-000001"
        ]
    );
    p.queue("20260101T000000.000002Z-000002", "a", "");
    p.queue("20260101T000000.000003Z-000003", "b", "");
    p.process(&["--exclude-machine", "a", "--exclude-machine", "b"])
        .assert()
        .success();
    assert_eq!(
        p.inbox(),
        [
            "20260101T000000.000002Z-000002.md",
            "20260101T000000.000003Z-000003.md"
        ]
    );
}

/// AC: `DECREE_MACHINES=a` in `.decree/.env` makes `process` run only `a`, `--machine b`
/// beats it, and no script sees `DECREE_MACHINES`.
#[test]
fn dotenv_sets_the_filter_a_flag_beats_it_and_scripts_never_see_it() {
    let p = Project::new();
    p.write(".env", "DECREE_MACHINES=a\nOTHER=1\n");
    p.queue("20260101T000000.000000Z-000000", "a", "");
    p.queue("20260101T000000.000001Z-000001", "b", "");
    p.process(&[]).assert().success();
    assert_eq!(p.order(), ["a 20260101T000000.000000Z-000000"]);
    assert_eq!(p.inbox(), ["20260101T000000.000001Z-000001.md"]);

    p.process(&["--machine", "b"]).assert().success();
    assert_eq!(
        p.order(),
        [
            "a 20260101T000000.000000Z-000000",
            "b 20260101T000000.000001Z-000001"
        ]
    );
    for id in [
        "20260101T000000.000000Z-000000",
        "20260101T000000.000001Z-000001",
    ] {
        let env = fs::read_to_string(p.tmp.path().join(format!("env-{id}.txt"))).unwrap();
        assert!(env.contains("OTHER=1\n"), "{env}");
        assert!(!env.contains("DECREE_MACHINES"), "{env}");
    }
}

/// The shell beats `.decree/.env`, and decree does not pass the shell's variable to scripts.
#[test]
fn the_shell_beats_dotenv() {
    let p = Project::new();
    p.write(".env", "DECREE_MACHINES=a\n");
    p.queue("20260101T000000.000000Z-000000", "a", "");
    p.queue("20260101T000000.000001Z-000001", "b", "");
    p.process(&[])
        .env("DECREE_EXCLUDE_MACHINES", "a")
        .assert()
        .success();
    assert_eq!(p.order(), ["b 20260101T000000.000001Z-000001"]);
    let env =
        fs::read_to_string(p.tmp.path().join("env-20260101T000000.000001Z-000001.txt")).unwrap();
    assert!(!env.contains("DECREE_EXCLUDE_MACHINES"), "{env}");
    assert!(!env.contains("DECREE_MACHINES"), "{env}");
}

/// AC: `--machine a --exclude-machine b` exits 2 and runs nothing; so do both variables.
#[test]
fn both_lists_exit_2_and_run_nothing() {
    let p = Project::new();
    p.queue("20260101T000000.000000Z-000000", "a", "");
    p.process(&["--machine", "a", "--exclude-machine", "b"])
        .assert()
        .code(2);
    p.decree_cmd(&["daemon", "--machine", "a", "--exclude-machine", "b"])
        .assert()
        .code(2);
    p.process(&[])
        .env("DECREE_MACHINES", "a")
        .env("DECREE_EXCLUDE_MACHINES", "b")
        .assert()
        .code(2);
    p.write(".env", "DECREE_MACHINES=a\nDECREE_EXCLUDE_MACHINES=b\n");
    p.process(&[]).assert().code(2);
    assert!(p.order().is_empty());
    assert!(p.runs().is_empty());
    assert_eq!(p.inbox(), ["20260101T000000.000000Z-000000.md"]);
}

/// A name that is not a machine is an error naming it; a built-in's name is not.
#[test]
fn an_unknown_machine_is_an_error_naming_it() {
    let p = Project::new();
    p.queue("20260101T000000.000000Z-000000", "a", "");
    let out = p.process(&["--machine", "a,nope"]).assert().code(1);
    let stderr = String::from_utf8_lossy(&out.get_output().stderr).into_owned();
    assert!(
        stderr.contains("--machine names `nope`, which is not a machine in .decree/machines/"),
        "{stderr}"
    );
    p.write(".env", "DECREE_EXCLUDE_MACHINES=zz\n");
    let out = p.process(&["--dry-run"]).assert().code(1);
    let stderr = String::from_utf8_lossy(&out.get_output().stderr).into_owned();
    assert!(
        stderr.contains("DECREE_EXCLUDE_MACHINES in .decree/.env names `zz`"),
        "{stderr}"
    );
    assert!(p.runs().is_empty());
    fs::remove_file(p.decree().join(".env")).unwrap();
    p.process(&["--exclude-machine", "develop"])
        .assert()
        .success();
    assert_eq!(p.order(), ["a 20260101T000000.000000Z-000000"]);
}

/// AC: migrations keep one order for every process. With `01` for `b` first, a process
/// that runs only `a` starts neither, says why and exits 0.
#[test]
fn a_migration_for_another_machine_holds_back_the_rest() {
    let p = Project::new();
    p.write("migrations/01-first.md", "---\nmachine: b\n---\n# First\n");
    p.write(
        "migrations/02-second.md",
        "---\nmachine: a\n---\n# Second\n",
    );
    let out = stdout(&mut p.process(&["--machine", "a"]));
    assert!(
        out.contains(
            "Held back: migration 01-first.md is for machine `b`, which this process does not run"
        ),
        "{out}"
    );
    assert!(p.runs().is_empty());

    // The process for `b` runs `01`, then is held back by `02`.
    let out = stdout(&mut p.process(&["--machine", "b"]));
    assert_eq!(p.order(), ["b 01-first"]);
    assert!(
        out.contains("Held back: migration 02-second.md is for machine `a`"),
        "{out}"
    );
    p.process(&["--machine", "a"]).assert().success();
    assert_eq!(p.order(), ["b 01-first", "a 02-second"]);
}

/// `--dry-run` lists only what this process would run.
#[test]
fn dry_run_honours_the_filter() {
    let p = Project::new();
    p.write("migrations/01-first.md", "---\nmachine: a\n---\n");
    p.write("migrations/02-second.md", "---\nmachine: b\n---\n");
    p.write("migrations/03-third.md", "---\nmachine: a\n---\n");
    p.queue("20260101T000000.000000Z-000000", "a", "");
    p.queue("20260101T000000.000001Z-000001", "b", "");
    let out = stdout(&mut p.process(&["--dry-run", "--machine", "a"]));
    assert!(out.contains("01-first.md"), "{out}");
    assert!(!out.contains("02-second.md  "), "{out}");
    assert!(!out.contains("03-third.md"), "{out}");
    assert!(out.contains("20260101T000000.000000Z-000000.md"), "{out}");
    assert!(!out.contains("20260101T000000.000001Z-000001.md"), "{out}");
    assert!(
        out.contains("Held back: migration 02-second.md is for machine `b`"),
        "{out}"
    );
    let json = stdout(&mut p.process(&["--dry-run", "--format", "json", "--exclude-machine", "a"]));
    let doc: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(doc["migrations"].as_array().unwrap().len(), 0, "{doc}");
    assert_eq!(
        doc["inbox"][0]["file"], "20260101T000000.000001Z-000001.md",
        "{doc}"
    );
    assert_eq!(doc["inbox"].as_array().unwrap().len(), 1, "{doc}");
    assert!(p.runs().is_empty());
}

/// `--retry <id>` of a run for a machine the process does not run is refused, naming it.
#[test]
fn retry_of_another_machines_run_is_refused() {
    let p = Project::new();
    p.queue(
        "20260101T000000.000000Z-000000",
        "b",
        "params: { fail: true }\n",
    );
    p.process(&[]).assert().code(1);
    let out = p
        .process(&[
            "--machine",
            "a",
            "--retry",
            "20260101T000000.000000Z-000000",
        ])
        .assert()
        .code(1);
    let stderr = String::from_utf8_lossy(&out.get_output().stderr).into_owned();
    assert!(
        stderr.contains("run 20260101T000000.000000Z-000000 is for machine `b`, which this process does not run"),
        "{stderr}"
    );
    let events = fs::read_to_string(
        p.decree()
            .join("runs/20260101T000000.000000Z-000000/events.jsonl"),
    )
    .unwrap();
    assert!(!events.contains("\"retry\""), "{events}");
}

/// A child run belongs to its parent: a process that runs `p` steps `p`'s child of machine
/// `b`, and leaves `b`'s own message alone.
#[test]
fn a_child_run_goes_with_its_parent() {
    let p = Project::new();
    p.queue("20260101T000000.000000Z-000000", "p", "");
    p.queue("20260101T000000.000001Z-000001", "b", "");
    p.process(&["--machine", "p"]).assert().success();
    let order = p.order();
    assert_eq!(order.len(), 2, "{order:?}");
    assert_eq!(order[0], "p 20260101T000000.000000Z-000000");
    assert!(order[1].starts_with("b "), "{order:?}");
    assert_eq!(p.inbox(), ["20260101T000000.000001Z-000001.md"]);
    let status = stdout(&mut p.decree_cmd(&["status", "20260101T000000.000000Z-000000"]));
    assert!(status.contains("done"), "{status}");
}

/// AC: every other `DECREE_*` key in `.decree/.env` is still rejected, and a machine's
/// `env_file` may not set the filter.
#[test]
fn check_still_rejects_other_decree_keys() {
    let p = Project::new();
    p.write(".gitignore", ".env*\n");
    p.write(".env", "DECREE_MACHINES=a\nDECREE_EXCLUDE_MACHINES=\n");
    p.decree_cmd(&["check"]).assert().success();
    p.write(".env", "DECREE_FOO=1\n");
    let out = p.decree_cmd(&["check"]).assert().code(1);
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).into_owned();
    assert!(
        stdout.contains(".env: line 1: key `DECREE_FOO` is reserved"),
        "{stdout}"
    );
    p.process(&[]).assert().code(1);
}

#[test]
fn a_machine_env_file_may_not_set_the_filter() {
    let p = Project::new();
    p.write(".gitignore", ".env*\n");
    let yml = fs::read_to_string(p.decree().join("machines/a.yml")).unwrap();
    p.write(
        "machines/a.yml",
        &yml.replace(
            "description: One script.\n",
            "description: One script.\nenv_file: .env.a\n",
        ),
    );
    p.write(".env.a", "DECREE_MACHINES=a\n");
    let out = p.decree_cmd(&["check"]).assert().code(1);
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).into_owned();
    assert!(
        stdout.contains(
            ".env.a: line 1: key `DECREE_MACHINES` is reserved: only `.decree/.env` sets it"
        ),
        "{stdout}"
    );
}
