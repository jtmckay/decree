//! The official `order` plugin (plugins/order.md, D86): at each daemon start (here `decree
//! daemon --once`), each machine's `order/` files run one at a time in file-name order, the
//! next only once the inbox is empty; each runs once, and again when it changes; a failing
//! file stops the ones after it. And its `emit_if_changed` scripts. Skipped without `jq`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

mod common;

fn has(tool: &str) -> bool {
    Command::new("sh")
        .args(["-c", &format!("command -v {tool}")])
        .output()
        .is_ok_and(|o| o.status.success())
}

/// A machine that logs the first heading of its message to `log.txt`; `emit` queues a
/// follow-up for `beta`, `fail` fails.
fn machine(name: &str) -> String {
    format!(
        "name: {name}\ndescription: Log what it ran.\ninitial: work\nstates:\n  work:\n    \
         invoke: work\n    emits: [beta]\n    transitions: {{ done: done }}\n  \
         done: {{ final: true }}\n  failed: {{ final: true }}\n"
    )
}

const WORK: &str = r##"#!/usr/bin/env bash
set -e
line=$(sed -n 's/^# //p' "$DECREE_MESSAGE" | head -1)
echo "$DECREE_MACHINE: $line" >> "$DECREE_PROJECT_ROOT/log.txt"
if [ "$line" = emit ]; then echo "# follow-up" | "$DECREE" emit --machine beta > /dev/null; fi
if [ "$line" = fail ]; then exit 1; fi
"##;

struct Project {
    tmp: TempDir,
}

impl Project {
    fn new() -> Option<Project> {
        if !has("jq") || !has("sha256sum") {
            eprintln!("jq or sha256sum missing: skipping");
            return None;
        }
        let p = Project {
            tmp: TempDir::new().unwrap(),
        };
        p.ok(&["init"]);
        p.ok(&["plugin", "add", "order"]);
        for m in ["alpha", "beta"] {
            common::write_file(&p.decree(&format!("machines/{m}/machine.yml")), machine(m));
            common::write_script(&p.decree(&format!("machines/{m}/scripts/work.sh")), WORK);
        }
        Some(p)
    }

    fn decree(&self, rel: &str) -> PathBuf {
        self.tmp.path().join(".decree").join(rel)
    }

    fn ok(&self, args: &[&str]) {
        common::decree_at(self.tmp.path())
            .env("NO_COLOR", "1")
            .args(args)
            .assert()
            .success();
    }

    fn order(&self, machine: &str, file: &str, text: &str) {
        common::write_file(
            &self.decree(&format!("machines/{machine}/order/{file}")),
            text,
        );
    }

    /// `decree daemon --once`: its exit code, and `log.txt`, which it empties.
    fn start(&self) -> (i32, String) {
        let out = common::decree_at(self.tmp.path())
            .env("NO_COLOR", "1")
            .args(["daemon", "--once", "-q"])
            .output()
            .unwrap();
        let log = self.tmp.path().join("log.txt");
        let text = fs::read_to_string(&log).unwrap_or_default();
        let _ = fs::remove_file(&log);
        (out.status.code().unwrap(), text)
    }

    fn record(&self, machine: &str) -> String {
        fs::read_to_string(self.decree(&format!("machines/{machine}/state/order.git.txt")))
            .unwrap_or_default()
    }
}

#[test]
fn order_runs_each_file_once_in_name_order_after_its_follow_ups() {
    let Some(p) = Project::new() else { return };
    p.order("alpha", "01-one.md", "# one\n");
    p.order("beta", "02-two.md", "---\nmachine: alpha\n---\n# emit\n");
    p.order("alpha", "03-three.md", "# three\n");
    let (code, log) = p.start();
    assert_eq!(code, 0, "{log}");
    // 02 names `alpha`; its follow-up for `beta` runs before 03 starts.
    assert_eq!(
        log,
        "alpha: one\nalpha: emit\nbeta: follow-up\nalpha: three\n"
    );
    assert!(
        p.record("alpha").starts_with("01-one.md "),
        "{}",
        p.record("alpha")
    );
    assert!(p.record("alpha").contains("\n03-three.md "));
    assert!(p.record("beta").starts_with("02-two.md "));

    // A second start runs nothing.
    assert_eq!(p.start(), (0, String::new()));

    // An edited file runs again; a failing one stops the files after it, until it passes.
    p.order("alpha", "01-one.md", "# one again\n");
    p.order("beta", "04-four.md", "# fail\n");
    p.order("alpha", "05-five.md", "# five\n");
    let (code, log) = p.start();
    assert_eq!((code, log.as_str()), (1, "alpha: one again\nbeta: fail\n"));
    p.order("beta", "04-four.md", "# four\n");
    let (code, log) = p.start();
    assert_eq!((code, log.as_str()), (0, "beta: four\nalpha: five\n"));
}

#[test]
fn order_max_runs_that_many_files_per_start() {
    let Some(p) = Project::new() else { return };
    p.order("alpha", "01-one.md", "# one\n");
    p.order("alpha", "02-two.md", "# two\n");
    let start = |p: &Project| {
        let out = common::decree_at(p.tmp.path())
            .env("ORDER_MAX", "1")
            .args(["daemon", "--once", "-q"])
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        let log = p.tmp.path().join("log.txt");
        let text = fs::read_to_string(&log).unwrap_or_default();
        let _ = fs::remove_file(&log);
        text
    };
    assert_eq!(start(&p), "alpha: one\n");
    assert_eq!(start(&p), "alpha: two\n");
    assert_eq!(start(&p), "");
}

/// git keeps each `order/` file, `graph.md` and `*.git.txt` state file, and none of what is
/// installed.
#[test]
fn git_keeps_order_graph_and_git_txt_files() {
    let Some(p) = Project::new() else { return };
    if !has("git") {
        return;
    }
    p.order("alpha", "01-one.md", "# one\n");
    p.ok(&["graph"]);
    assert_eq!(p.start().0, 0);
    let ignored = |rel: &str| {
        Command::new("git")
            .args(["check-ignore", "-q", rel])
            .current_dir(p.tmp.path().join(".decree"))
            .status()
            .map(|s| s.success())
            .unwrap()
    };
    Command::new("git")
        .args(["init", "-q"])
        .current_dir(p.tmp.path())
        .status()
        .unwrap();
    for rel in [
        "machines/alpha/order/01-one.md",
        "machines/alpha/state/order.git.txt",
        "machines/alpha/graph.md",
    ] {
        assert!(Path::new(&p.decree(rel)).exists(), "{rel}");
        assert!(!ignored(rel), "{rel} is ignored");
    }
    for rel in [
        "machines/alpha/machine.yml",
        "machines/alpha/scripts/work.sh",
        "machines/order/scripts/next.sh",
        "machines/order/state/running",
    ] {
        assert!(ignored(rel), "{rel} is not ignored");
    }
}

/// `emit_if_changed` queues a message once, and again only when it changes;
/// `emit_if_changed_git` keeps its hash in a `.git.txt` file.
#[test]
fn emit_if_changed_queues_a_message_only_when_it_changed() {
    let Some(p) = Project::new() else { return };
    let script = p.decree("machines/order/scripts/emit_if_changed.sh");
    let state = p.decree("machines/alpha/state");
    let emit = |script: &Path, key: &str, text: &str| {
        let out = Command::new(script)
            .args([key, "--machine", "alpha"])
            .current_dir(p.tmp.path())
            .env("DECREE_STATE_DIR", &state)
            .env("DECREE", env!("CARGO_BIN_EXE_decree"))
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write;
                child.stdin.take().unwrap().write_all(text.as_bytes())?;
                child.wait_with_output()
            })
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        String::from_utf8(out.stdout).unwrap()
    };
    let inbox = || fs::read_dir(p.decree("inbox")).unwrap().count();
    emit(&script, "weekly", "# Week 1\n");
    assert_eq!(inbox(), 1);
    let out = emit(&script, "weekly", "# Week 1\n");
    assert!(out.contains("unchanged"), "{out}");
    assert_eq!(inbox(), 1);
    emit(&script, "weekly", "# Week 2\n");
    assert_eq!(inbox(), 2);
    assert!(state.join("weekly.txt").is_file());

    let git = p.decree("machines/order/scripts/emit_if_changed_git.sh");
    emit(&git, "release", "# v1\n");
    emit(&git, "release", "# v1\n");
    assert_eq!(inbox(), 3);
    assert!(state.join("release.git.txt").is_file());
}
