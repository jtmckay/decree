//! A machine's state folder, `.decree/machines/<machine>/state/` (docs/reference/scripts.md, State): every
//! script gets its own machine's folder as `DECREE_STATE_DIR`, a child machine's scripts the child's,
//! decree creates it before the script runs, what a script keeps there survives the next run,
//! and deleting runs leaves it. Each test builds its own `.decree/` in a temp directory and
//! drives the `decree` binary.

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
state_files:
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
description: Say where its state folder is.
initial: work
states:
  work:
    invoke: where
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
";

/// Fails unless the state folder exists; logs the machine, phase and state folder.
const WHERE: &str = r#"#!/usr/bin/env bash
[ -d "$DECREE_STATE_DIR" ] || exit 1
echo "$DECREE_MACHINE $DECREE_PHASE $DECREE_STATE_DIR" >> "$DECREE_PROJECT_ROOT/where.log"
"#;

/// Adds one to `$DECREE_STATE_DIR/count`.
const COUNT: &str = r#"#!/usr/bin/env bash
n=$(cat "$DECREE_STATE_DIR/count" 2>/dev/null || echo 0)
echo $((n + 1)) > "$DECREE_STATE_DIR/count"
echo "$DECREE_MACHINE invoke $DECREE_STATE_DIR" >> "$DECREE_PROJECT_ROOT/where.log"
"#;

fn project() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let decree = tmp.path().join(".decree");
    for dir in ["machines", "scripts", "inbox", "runs"] {
        fs::create_dir_all(decree.join(dir)).unwrap();
    }
    fs::write(decree.join("processed.md"), "").unwrap();
    common::write_file(&decree.join("machines/parent/machine.yml"), PARENT);
    common::write_file(&decree.join("machines/child/machine.yml"), CHILD);
    write_script(&decree.join("scripts/where"), WHERE);
    write_script(&decree.join("scripts/count"), COUNT);
    tmp
}

fn decree(root: &Path, args: &[&str]) -> Command {
    let mut cmd = common::decree_at(root);
    cmd.env("NO_COLOR", "1").args(args);
    cmd
}

/// One run of `parent`, and its child, to the end.
fn run(root: &Path, n: usize) {
    fs::write(
        root.join(format!(".decree/inbox/run-{n}.md")),
        "---\nmachine: parent\n---\nCount.\n",
    )
    .unwrap();
    decree(root, &["daemon", "--once"]).assert().code(0);
}

#[test]
fn each_script_gets_its_own_machines_state_and_a_child_the_childs() {
    let tmp = project();
    let root = tmp.path().canonicalize().unwrap();
    run(&root, 1);
    let parent = root.join(".decree/machines/parent/state");
    let child = root.join(".decree/machines/child/state");
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
fn the_state_survives_runs_and_their_folders() {
    let tmp = project();
    let root = tmp.path();
    let count = root.join(".decree/machines/parent/state/count");
    run(root, 1);
    assert_eq!(fs::read_to_string(&count).unwrap(), "1\n");
    run(root, 2);
    assert_eq!(fs::read_to_string(&count).unwrap(), "2\n");

    fs::remove_dir_all(root.join(".decree/runs")).unwrap();
    fs::create_dir_all(root.join(".decree/runs")).unwrap();
    assert_eq!(fs::read_to_string(&count).unwrap(), "2\n");
    assert!(root.join(".decree/machines/child/state").is_dir());

    // Declared, so `decree check` has nothing to say about `count`; the child declares
    // nothing, so its empty folder is fine too.
    let out = decree(root, &["check"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("/state/"), "{stderr}");
}
