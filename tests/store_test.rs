//! A machine's store, `.decree/store/<machine>/` (docs/reference/scripts.md, Store): every
//! script gets its own machine's folder as `DECREE_STORE`, a child machine's scripts the child's,
//! decree creates it before the script runs, what a script keeps there survives the next run,
//! and `decree prune` leaves it. Each test builds its own `.decree/` in a temp directory and
//! drives the `decree` binary.

use assert_cmd::cargo::cargo_bin_cmd;
use assert_cmd::Command;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

mod common;
use common::write_script;

/// Counts its runs in `count`, from a root `onentry`, an invoke and an `onexit`, then runs
/// `child` as a child run.
const PARENT: &str = "\
name: parent
description: Count runs, then run the child.
store:
  count: How many runs there have been. work increments it.
onentry: [where]
initial: work
states:
  work:
    invoke: count
    onexit: [where]
    transitions: { done: sub }
  sub:
    invoke: { machine: child }
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
";

const CHILD: &str = "\
name: child
description: Say where its store is.
initial: work
states:
  work:
    invoke: where
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
";

/// Fails unless the store folder exists; logs the machine, phase and store.
const WHERE: &str = r#"#!/usr/bin/env bash
[ -d "$DECREE_STORE" ] || exit 1
echo "$DECREE_MACHINE $DECREE_PHASE $DECREE_STORE" >> "$DECREE_PROJECT_ROOT/where.log"
"#;

/// Adds one to `$DECREE_STORE/count`.
const COUNT: &str = r#"#!/usr/bin/env bash
n=$(cat "$DECREE_STORE/count" 2>/dev/null || echo 0)
echo $((n + 1)) > "$DECREE_STORE/count"
echo "$DECREE_MACHINE invoke $DECREE_STORE" >> "$DECREE_PROJECT_ROOT/where.log"
"#;

fn project() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let decree = tmp.path().join(".decree");
    for dir in ["machines", "scripts", "migrations", "inbox", "runs"] {
        fs::create_dir_all(decree.join(dir)).unwrap();
    }
    fs::write(decree.join("processed.md"), "").unwrap();
    fs::write(decree.join("machines/parent.yml"), PARENT).unwrap();
    fs::write(decree.join("machines/child.yml"), CHILD).unwrap();
    write_script(&decree.join("scripts/where"), WHERE);
    write_script(&decree.join("scripts/count"), COUNT);
    tmp
}

fn decree(root: &Path, args: &[&str]) -> Command {
    let mut cmd = cargo_bin_cmd!("decree");
    cmd.current_dir(root).env("NO_COLOR", "1").args(args);
    cmd
}

/// One run of `parent`, and its child, to the end.
fn run(root: &Path, n: usize) {
    fs::write(
        root.join(format!(".decree/inbox/run-{n}.md")),
        "---\nmachine: parent\n---\nCount.\n",
    )
    .unwrap();
    decree(root, &["process"]).assert().code(0);
}

#[test]
fn each_script_gets_its_own_machines_store_and_a_child_the_childs() {
    let tmp = project();
    let root = tmp.path().canonicalize().unwrap();
    run(&root, 1);
    let parent = root.join(".decree/store/parent");
    let child = root.join(".decree/store/child");
    let log = fs::read_to_string(root.join("where.log")).unwrap();
    assert_eq!(
        log.lines().collect::<Vec<_>>(),
        [
            format!("parent onentry {}", parent.display()),
            format!("parent invoke {}", parent.display()),
            format!("parent onexit {}", parent.display()),
            format!("child invoke {}", child.display()),
        ]
    );
    assert!(child.is_dir());
}

#[test]
fn the_store_survives_runs_and_prune() {
    let tmp = project();
    let root = tmp.path();
    let count = root.join(".decree/store/parent/count");
    run(root, 1);
    assert_eq!(fs::read_to_string(&count).unwrap(), "1\n");
    run(root, 2);
    assert_eq!(fs::read_to_string(&count).unwrap(), "2\n");

    decree(root, &["prune", "--older-than", "0s"])
        .assert()
        .code(0);
    let runs = fs::read_dir(root.join(".decree/runs")).unwrap().count();
    assert_eq!(runs, 0, "prune deleted the finished runs");
    assert_eq!(fs::read_to_string(&count).unwrap(), "2\n");
    assert!(root.join(".decree/store/child").is_dir());

    // Declared, so `decree check` has nothing to say about `count`; the child declares
    // nothing, so its empty folder is fine too.
    let out = decree(root, &["check"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("store/"), "{stderr}");
}
