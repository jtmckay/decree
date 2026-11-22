//! Machines run at once, each up to its `concurrency` (docs/reference/machines.md,
//! Concurrency; D89): a machine at its limit keeps its messages queued and leaves the
//! others alone; a run waiting holds no slot; a child run takes a slot of its own machine.
//! Each test is a fresh project without the default plugins, whose scripts log when they
//! start and end to `log.txt`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::thread;
use std::time::{Duration, Instant};

use tempfile::TempDir;

mod common;

struct Project {
    tmp: TempDir,
}

/// A machine that runs `work` once, at most `concurrency` runs at a time (none: the
/// default).
fn machine(name: &str, concurrency: Option<u32>) -> String {
    let concurrency = concurrency.map_or(String::new(), |n| format!("concurrency: {n}\n"));
    format!(
        "name: {name}\ndescription: Log, sleep, log.\n{concurrency}initial: work\nstates:\n  \
         work:\n    invoke: work\n    transitions: {{ done: done }}\n  done: {{ final: true }}\n  \
         failed: {{ final: true }}\n"
    )
}

/// `work`: logs `start`, prints two lines `seconds` apart, logs `end`.
fn work(seconds: f32) -> String {
    format!(
        "#!/usr/bin/env bash\n\
         log() {{ echo \"$1 $DECREE_MACHINE $DECREE_MESSAGE_ID $(date +%s%N)\" >> \"$DECREE_PROJECT_ROOT/log.txt\"; }}\n\
         log start\necho first line\nsleep {seconds}\necho second line\nlog end\n"
    )
}

impl Project {
    fn new() -> Project {
        let p = Project {
            tmp: TempDir::new().unwrap(),
        };
        let out = common::std_decree_at(p.root())
            .arg("init")
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        common::drop_default_plugins(p.root());
        p
    }

    fn root(&self) -> &Path {
        self.tmp.path()
    }

    fn decree(&self, rel: &str) -> PathBuf {
        self.root().join(".decree").join(rel)
    }

    fn machine(&self, name: &str, text: &str, script: &str) {
        common::write_file(&self.decree(&format!("machines/{name}/machine.yml")), text);
        common::write_script(
            &self.decree(&format!("machines/{name}/scripts/work.sh")),
            script,
        );
    }

    fn emit(&self, machine: &str) -> String {
        let out = common::std_decree_at(self.root())
            .args(["emit", "--machine", machine])
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    }

    /// `decree daemon --once`: (exit code, stdout).
    fn once(&self) -> (i32, String) {
        let out = common::std_decree_at(self.root())
            .env("NO_COLOR", "1")
            .args(["daemon", "--once"])
            .output()
            .unwrap();
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8(out.stdout).unwrap(),
        )
    }

    /// When each run started and ended, in ns, from `log.txt`.
    fn times(&self) -> BTreeMap<String, (u128, u128)> {
        let text = fs::read_to_string(self.root().join("log.txt")).unwrap_or_default();
        let mut times: BTreeMap<String, (u128, u128)> = BTreeMap::new();
        for line in text.lines() {
            let w: Vec<&str> = line.split(' ').collect();
            let t: u128 = w[3].parse().unwrap();
            let entry = times.entry(w[2].to_string()).or_default();
            if w[0] == "start" {
                entry.0 = t;
            } else {
                entry.1 = t;
            }
        }
        times
    }

    fn final_state(&self, id: &str) -> String {
        let events = fs::read_to_string(self.decree(&format!("runs/{id}/events.jsonl"))).unwrap();
        let last: serde_json::Value = serde_json::from_str(events.lines().last().unwrap()).unwrap();
        last["state"].as_str().unwrap_or_default().to_string()
    }
}

/// Whether the runs `a` and `b` executed at the same time.
fn overlap(times: &BTreeMap<String, (u128, u128)>, a: &str, b: &str) -> bool {
    let (a, b) = (times[a], times[b]);
    a.0 < b.1 && b.0 < a.1
}

/// AC: a slow machine does not hold up another: `fast`, queued after `slow`, runs and
/// finishes while `slow` still runs.
#[test]
fn machines_do_not_block_each_other() {
    let p = Project::new();
    p.machine("slow", &machine("slow", None), &work(2.0));
    p.machine("fast", &machine("fast", None), &work(0.0));
    let slow = p.emit("slow");
    let fast = p.emit("fast");
    let (code, out) = p.once();
    assert_eq!(code, 0, "{out}");
    let t = p.times();
    assert!(t[&fast].1 < t[&slow].1, "fast waited for slow: {t:?}");
    assert!(overlap(&t, &slow, &fast), "{t:?}");
}

/// AC: with the default `concurrency` (1), two runs of one machine run one after the other;
/// with `concurrency: 2`, at once.
#[test]
fn concurrency_limits_the_runs_of_one_machine() {
    let p = Project::new();
    p.machine("one", &machine("one", None), &work(0.5));
    p.machine("two", &machine("two", Some(2)), &work(0.5));
    let one = [p.emit("one"), p.emit("one")];
    let two = [p.emit("two"), p.emit("two")];
    let (code, out) = p.once();
    assert_eq!(code, 0, "{out}");
    let t = p.times();
    assert!(
        !overlap(&t, &one[0], &one[1]),
        "one ran twice at once: {t:?}"
    );
    assert!(t[&one[0]].1 <= t[&one[1]].0, "out of order: {t:?}");
    assert!(
        overlap(&t, &two[0], &two[1]),
        "two ran one at a time: {t:?}"
    );
}

/// AC: a machine at its limit keeps its next message in `inbox/`, unclaimed, until a slot
/// frees.
#[test]
fn a_full_machine_keeps_its_messages_queued() {
    let p = Project::new();
    p.machine("one", &machine("one", None), &work(1.5));
    let first = p.emit("one");
    let second = p.emit("one");
    let mut daemon = common::std_decree_at(p.root())
        .args(["daemon", "--once", "--quiet"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let running = p.decree(&format!("runs/{first}/.running"));
    let deadline = Instant::now() + Duration::from_secs(20);
    while !running.exists() {
        assert!(Instant::now() < deadline, "the first run never started");
        thread::sleep(Duration::from_millis(10));
    }
    thread::sleep(Duration::from_millis(300));
    assert!(p.decree(&format!("inbox/{second}.md")).is_file());
    assert!(!p.decree(&format!("runs/{second}")).exists());
    assert!(daemon.wait().unwrap().success());
    assert_eq!(p.final_state(&second), "done");
}

/// AC: a run waiting for a reply holds no slot: with `concurrency` 1, a second run of the
/// machine starts and reaches its own wait.
#[test]
fn a_waiting_run_holds_no_slot() {
    let p = Project::new();
    let asker = "name: asker\ndescription: Ask.\ninitial: ask\nstates:\n  ask:\n    invoke:\n      \
                 reply:\n        question: Go?\n        notify: work\n    transitions:\n      \
                 go: { target: done, description: Go. }\n      stop: { target: done, description: Stop. }\n  \
                 done: { final: true }\n  failed: { final: true }\n";
    p.machine("asker", asker, &work(0.0));
    let a = p.emit("asker");
    let b = p.emit("asker");
    let (_, out) = p.once();
    for id in [&a, &b] {
        assert!(out.contains(&format!("run {id} in `ask`")), "{out}");
    }
    for id in [&a, &b] {
        let out = common::std_decree_at(p.root())
            .args(["reply", id, "go"])
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
    }
    let (code, out) = p.once();
    assert_eq!(code, 0, "{out}");
    assert_eq!(p.final_state(&a), "done");
    assert_eq!(p.final_state(&b), "done");
}

/// AC: child runs take a slot of their own machine: two `parent` runs (2 at once) each
/// invoke `child` (1 at once), whose runs never overlap; both parents finish.
#[test]
fn a_child_run_takes_a_slot_of_its_own_machine() {
    let p = Project::new();
    let parent = "name: parent\ndescription: Invoke child.\nconcurrency: 2\ninitial: call\n\
                  states:\n  call:\n    invoke: { machine: child }\n    transitions: { done: done }\n  \
                  done: { final: true }\n  failed: { final: true }\n";
    common::write_file(&p.decree("machines/parent/machine.yml"), parent);
    p.machine("child", &machine("child", None), &work(0.5));
    let parents = [p.emit("parent"), p.emit("parent")];
    let (code, out) = p.once();
    assert_eq!(code, 0, "{out}");
    for id in &parents {
        assert_eq!(p.final_state(id), "done", "{out}");
    }
    let t = p.times();
    let children: Vec<&String> = t.keys().collect();
    assert_eq!(children.len(), 2, "{t:?}");
    assert!(!overlap(&t, children[0], children[1]), "{t:?}");
}

/// AC: a finished child continues its parent without holding its own slot, so parents and
/// children with one slot each never wait for each other: two parents whose children wait
/// for a reply, both answered, both finish.
#[test]
fn a_finished_child_continues_its_parent() {
    let p = Project::new();
    let parent = "name: parent\ndescription: Invoke asker.\ninitial: call\nstates:\n  call:\n    \
                  invoke: { machine: asker }\n    transitions: { done: done }\n  \
                  done: { final: true }\n  failed: { final: true }\n";
    let asker = "name: asker\ndescription: Ask.\ninitial: ask\nstates:\n  ask:\n    invoke:\n      \
                 reply:\n        question: Go?\n        notify: work\n    transitions:\n      \
                 go: { target: done, description: Go. }\n      stop: { target: done, description: Stop. }\n  \
                 done: { final: true }\n  failed: { final: true }\n";
    common::write_file(&p.decree("machines/parent/machine.yml"), parent);
    p.machine("asker", asker, &work(0.3));
    let parents = [p.emit("parent"), p.emit("parent")];
    p.once();
    let children: Vec<String> = p.times().into_keys().collect();
    assert_eq!(children.len(), 2);
    for id in &children {
        let out = common::std_decree_at(p.root())
            .args(["reply", id, "go"])
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
    }
    let (code, out) = p.once();
    assert_eq!(code, 0, "{out}");
    for id in &parents {
        assert_eq!(p.final_state(id), "done", "{out}");
    }
}

/// AC: while runs execute at once, each line of their output is behind its run's prefix,
/// `[<last six characters of its id>] `.
#[test]
fn output_of_runs_at_once_is_prefixed() {
    let p = Project::new();
    p.machine("a", &machine("a", None), &work(1.0));
    p.machine("b", &machine("b", None), &work(1.0));
    let a = p.emit("a");
    let b = p.emit("b");
    let (code, out) = p.once();
    assert_eq!(code, 0, "{out}");
    for id in [&a, &b] {
        let prefix = format!("[{}] ", &id[id.len() - 6..]);
        assert!(
            out.lines()
                .any(|l| l.starts_with(&prefix) && l.ends_with("second line")),
            "{out}"
        );
        assert!(out.contains(&format!("✓ {id} done in ")), "{out}");
    }
}

/// AC: SIGTERM stops every script running, and each run is `interrupted`.
#[test]
fn a_signal_interrupts_every_run() {
    let p = Project::new();
    p.machine("a", &machine("a", None), &work(30.0));
    p.machine("b", &machine("b", None), &work(30.0));
    let ids = [p.emit("a"), p.emit("b")];
    let mut daemon = common::std_decree_at(p.root())
        .args(["daemon", "--interval", "1s", "--quiet"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while !ids
        .iter()
        .all(|id| p.decree(&format!("runs/{id}/.running")).exists())
    {
        assert!(Instant::now() < deadline, "both never ran at once");
        thread::sleep(Duration::from_millis(10));
    }
    // SAFETY: sends SIGTERM to the daemon this test spawned.
    unsafe { libc::kill(daemon.id() as i32, libc::SIGTERM) };
    let deadline = Instant::now() + Duration::from_secs(20);
    while daemon.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline, "the daemon did not stop");
        thread::sleep(Duration::from_millis(20));
    }
    for id in &ids {
        let out = common::std_decree_at(p.root())
            .args(["status", id, "--format", "json"])
            .output()
            .unwrap();
        let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(doc["status"], "interrupted", "{doc}");
    }
}

/// AC: machines in one `concurrency_group` share its slots: `a` and `b`, both in group
/// `tree` with one slot, never run at once, while `c`, outside it, runs beside them.
#[test]
fn a_concurrency_group_shares_its_slots() {
    let p = Project::new();
    let grouped = |name: &str| {
        machine(name, None).replace("initial: work", "concurrency_group: tree\ninitial: work")
    };
    p.machine("a", &grouped("a"), &work(0.5));
    p.machine("b", &grouped("b"), &work(0.5));
    p.machine("c", &machine("c", None), &work(0.5));
    let a = p.emit("a");
    let b = p.emit("b");
    let c = p.emit("c");
    let (code, out) = p.once();
    assert_eq!(code, 0, "{out}");
    let t = p.times();
    assert!(!overlap(&t, &a, &b), "a and b ran at once: {t:?}");
    assert!(
        overlap(&t, &a, &c) || overlap(&t, &b, &c),
        "c waited: {t:?}"
    );
}
