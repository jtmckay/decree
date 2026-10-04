//! 0.4.2's hooks as machines (docs/decisions.md, D15): `beforeAll` is root
//! `onentry`, `afterAll` root `onexit`, `beforeEach` and `afterEach` the atomic state's
//! `onentry` and `onexit`, and `onDeadLetter` `failed`'s `onentry`. They run once per
//! visit to a state; `max_attempts` re-runs only the invoke. Each test builds its own
//! `.decree/` in a temp directory.

use assert_cmd::cargo::cargo_bin_cmd;
use assert_cmd::Command;
use serde_json::Value;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const MACHINE: &str = "\
name: hooks
description: 0.4.2's five hooks around one invoke.
data:
  succeed_on: { type: int, default: 1 }
  fail: { type: string, default: none }
onentry: [before_all]
onexit: [after_all]
initial: work
states:
  work:
    onentry: [before_each]
    onexit: [after_each]
    invoke: work
    max_attempts: 2
    transitions: { done: done }
  done: { final: true, onentry: [commit] }
  failed: { final: true, onentry: [on_dead_letter] }
";

/// Every script: appends `<state> <phase> <script> <attempt>` to `order.log`. `work`
/// fails until attempt `succeed_on`; any script exits 1 when the `fail` param names it.
const RECORD: &str = r#"#!/usr/bin/env bash
name=$(basename "$0")
echo "$DECREE_STATE $DECREE_PHASE $name $DECREE_ATTEMPT" >> "$DECREE_PROJECT_ROOT/order.log"
if [ "$DECREE_DATA_FAIL" = "$name" ]; then exit 1; fi
if [ "$name" = work ] && [ "$DECREE_ATTEMPT" -lt "$DECREE_DATA_SUCCEED_ON" ]; then exit 1; fi
exit 0
"#;

const SCRIPTS: &[&str] = &[
    "before_all",
    "before_each",
    "work",
    "after_each",
    "after_all",
    "commit",
    "on_dead_letter",
];

const ON_DEAD_LETTER: &str = "failed onentry on_dead_letter 1";

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
        fs::write(decree.join("machines/hooks.yml"), MACHINE).unwrap();
        for name in SCRIPTS {
            let path = decree.join("scripts").join(name);
            fs::write(&path, RECORD).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        Project { tmp }
    }

    fn decree(&self) -> PathBuf {
        self.tmp.path().join(".decree")
    }

    /// Queue `run.md` with `params`, run `decree process`, and return its exit code.
    fn run(&self, params: &str) -> i32 {
        fs::write(
            self.decree().join("inbox/run.md"),
            format!("---\nmachine: hooks\nparams: {{ {params} }}\n---\n"),
        )
        .unwrap();
        let output = self.process().output().unwrap();
        output.status.code().unwrap()
    }

    fn order(&self) -> Vec<String> {
        fs::read_to_string(self.tmp.path().join("order.log"))
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect()
    }

    /// The events of the one run.
    fn events(&self) -> Vec<Value> {
        let runs: Vec<PathBuf> = fs::read_dir(self.decree().join("runs"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert_eq!(runs.len(), 1, "{runs:?}");
        events(&runs[0])
    }

    fn process(&self) -> Command {
        let mut cmd = cargo_bin_cmd!("decree");
        cmd.current_dir(self.tmp.path())
            .env("NO_COLOR", "1")
            .arg("process");
        cmd
    }
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

fn dead_letters(order: &[String]) -> usize {
    order.iter().filter(|l| *l == ON_DEAD_LETTER).count()
}

// ---------------------------------------------------------------
// Order: root onentry, onentry, invoke, onexit, root onexit
// ---------------------------------------------------------------

#[test]
fn hooks_run_root_onentry_onentry_invoke_onexit_root_onexit() {
    let p = Project::new();
    assert_eq!(p.run(""), 0);
    assert_eq!(
        p.order(),
        [
            "_root onentry before_all 1",
            "work onentry before_each 1",
            "work invoke work 1",
            "work onexit after_each 1",
            "done onentry commit 1",
            "_root onexit after_all 1",
        ]
    );
    assert_eq!(last_state(&p.events()), "done");
}

#[test]
fn two_attempts_run_the_invoke_twice_between_one_onentry_and_one_onexit() {
    let p = Project::new();
    assert_eq!(p.run("succeed_on: 2"), 0);
    assert_eq!(
        p.order(),
        [
            "_root onentry before_all 1",
            "work onentry before_each 1",
            "work invoke work 1",
            "work invoke work 2",
            "work onexit after_each 1",
            "done onentry commit 1",
            "_root onexit after_all 1",
        ]
    );
    // The retry is one `attempt` transition that stays in `work`.
    let events = p.events();
    let attempts: Vec<&Value> = events.iter().filter(|e| e["source"] == "attempt").collect();
    assert_eq!(attempts.len(), 1, "{events:?}");
    assert_eq!(attempts[0]["event"], "error");
    assert_eq!(attempts[0]["from"], "work");
    assert_eq!(attempts[0]["to"], "work");
}

// ---------------------------------------------------------------
// onDeadLetter: `failed`'s onentry, exactly once
// ---------------------------------------------------------------

/// 0.4.2's `test_on_dead_letter_hook_fires_on_exhaustion`.
#[test]
fn failed_onentry_runs_once_when_attempts_run_out() {
    let p = Project::new();
    assert_eq!(p.run("succeed_on: 3"), 1);
    assert_eq!(
        p.order(),
        [
            "_root onentry before_all 1",
            "work onentry before_each 1",
            "work invoke work 1",
            "work invoke work 2",
            "work onexit after_each 1",
            ON_DEAD_LETTER,
            "_root onexit after_all 1",
        ]
    );
    assert_eq!(last_state(&p.events()), "failed");
}

/// 0.4.2's `test_on_dead_letter_hook_does_not_fire_on_before_each_failure`, reversed on
/// purpose: a failing `onentry` skips the invoke, and `failed`'s `onentry` runs once.
#[test]
fn failed_onentry_runs_once_after_a_failing_onentry() {
    let p = Project::new();
    assert_eq!(p.run("fail: before_each"), 1);
    let order = p.order();
    assert!(!order.iter().any(|l| l.contains(" invoke ")), "{order:?}");
    assert_eq!(
        order,
        [
            "_root onentry before_all 1",
            "work onentry before_each 1",
            "work onexit after_each 1",
            ON_DEAD_LETTER,
            "_root onexit after_all 1",
        ]
    );
    assert_eq!(last_state(&p.events()), "failed");
}

/// A failing root `onentry` (0.4.2's `beforeAll`) is `error` from `work`, which does not
/// handle it: `work`'s own `onentry` still runs, its invoke does not, and the run fails.
#[test]
fn failed_onentry_runs_once_after_a_failing_root_onentry() {
    let p = Project::new();
    assert_eq!(p.run("fail: before_all"), 1);
    assert_eq!(
        p.order(),
        [
            "_root onentry before_all 1",
            "work onentry before_each 1",
            "work onexit after_each 1",
            ON_DEAD_LETTER,
            "_root onexit after_all 1",
        ]
    );
    assert_eq!(last_state(&p.events()), "failed");
}

/// A failing `onentry` of a final state other than `failed` moves the run to `failed`.
#[test]
fn failed_onentry_runs_once_after_a_failing_final_onentry() {
    let p = Project::new();
    assert_eq!(p.run("fail: commit"), 1);
    assert_eq!(
        p.order(),
        [
            "_root onentry before_all 1",
            "work onentry before_each 1",
            "work invoke work 1",
            "work onexit after_each 1",
            "done onentry commit 1",
            ON_DEAD_LETTER,
            "_root onexit after_all 1",
        ]
    );
    assert_eq!(last_state(&p.events()), "failed");
}

/// A failing `onentry` on `failed` itself is only logged: it is not run again.
#[test]
fn a_failing_failed_onentry_runs_once() {
    let p = Project::new();
    assert_eq!(p.run("fail: on_dead_letter, succeed_on: 3"), 1);
    let order = p.order();
    assert_eq!(dead_letters(&order), 1, "{order:?}");
    assert_eq!(order.last().unwrap(), "_root onexit after_all 1");
    assert_eq!(last_state(&p.events()), "failed");
}

/// A run that ends `done` never runs `failed`'s `onentry`.
#[test]
fn a_run_that_ends_done_runs_no_failed_onentry() {
    let p = Project::new();
    assert_eq!(p.run("succeed_on: 2"), 0);
    assert_eq!(dead_letters(&p.order()), 0);
}

// ---------------------------------------------------------------
// git_baseline and snapshot, as `decree init` writes them
// ---------------------------------------------------------------

/// Root `onentry` records the baseline; `implement`'s `onentry` snapshots each round.
const GIT_MACHINE: &str = "\
name: git_rounds
description: Two rounds of edits between snapshots.
onentry: [git_baseline]
initial: implement
states:
  implement:
    onentry: [snapshot]
    invoke: edit
    transitions: { done: done, again: implement }
  done: { final: true }
  failed: { final: true }
";

/// Round 1 commits the dirty tree (so `HEAD` moves), dirties it again and asks for
/// another round; round 2 is done.
const EDIT: &str = r#"#!/usr/bin/env bash
set -euo pipefail
if [ "$DECREE_VISITS" = 1 ]; then
  git -c commit.gpgsign=false commit -qam "round 1"
  echo "round 2" >> notes.txt
  echo '{"event": "again"}'
fi
"#;

fn git(dir: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}: {output:?}");
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn git_baseline_is_written_once_and_snapshot_stores_one_stash_per_visit() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    git(root, &["init", "-q"]);
    git(root, &["config", "user.email", "decree@example.com"]);
    git(root, &["config", "user.name", "decree"]);
    fs::write(root.join("notes.txt"), "start\n").unwrap();
    git(root, &["add", "notes.txt"]);
    git(
        root,
        &["-c", "commit.gpgsign=false", "commit", "-qm", "start"],
    );
    let start = git(root, &["rev-parse", "HEAD"]);

    cargo_bin_cmd!("decree")
        .current_dir(root)
        .env("NO_COLOR", "1")
        .args(["init", "--ai", "claude"])
        .assert()
        .success();
    let decree = root.join(".decree");
    fs::write(decree.join("machines/git_rounds.yml"), GIT_MACHINE).unwrap();
    let edit = decree.join("scripts/git_rounds/edit");
    fs::create_dir_all(edit.parent().unwrap()).unwrap();
    fs::write(&edit, EDIT).unwrap();
    fs::set_permissions(&edit, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(
        decree.join("inbox/rounds.md"),
        "---\nid: rounds\nmachine: git_rounds\n---\n",
    )
    .unwrap();
    // A dirty tree, so the first visit has something to snapshot.
    fs::write(root.join("notes.txt"), "start\nround 1\n").unwrap();

    cargo_bin_cmd!("decree")
        .current_dir(root)
        .env("NO_COLOR", "1")
        .arg("process")
        .assert()
        .success();
    let run = decree.join("runs/rounds");
    assert_eq!(last_state(&events(&run)), "done");

    // The baseline is the HEAD the run started from, though round 1 committed.
    let baseline = run.join("baseline");
    assert_eq!(fs::read_to_string(&baseline).unwrap(), start);
    assert_ne!(git(root, &["rev-parse", "HEAD"]), start);

    // One stash per visit to `implement`, each holding that visit's tree.
    assert_eq!(
        git(root, &["stash", "list", "--format=%gs"]),
        "decree rounds round 2\ndecree rounds round 1\n"
    );
    assert_eq!(
        git(root, &["show", "stash@{1}:notes.txt"]),
        "start\nround 1\n"
    );
    assert_eq!(
        git(root, &["show", "stash@{0}:notes.txt"]),
        "start\nround 1\nround 2\n"
    );

    // Running git_baseline again (as `decree retry` does) keeps the first baseline.
    let output = std::process::Command::new(decree.join("scripts/git_baseline.sh"))
        .current_dir(root)
        .env("DECREE_RUN_DIR", &run)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("baseline {start}")
    );
    assert_eq!(fs::read_to_string(&baseline).unwrap(), start);
}
