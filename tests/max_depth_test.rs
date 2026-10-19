//! `DECREE_MAX_DEPTH` (docs/reference/cli.md, Environment; docs/decisions.md): `max_depth`
//! defaults to 10, and the shell or `.decree/.env` can change it for `process`, `daemon`,
//! `emit` and `check`.

use assert_cmd::cargo::cargo_bin_cmd;
use assert_cmd::Command;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

mod common;
use common::write_script;

/// `chain` emits to itself; `nest` emits to itself up to depth 2, then invokes `leaf` as a
/// child run.
const CHAIN: &str = "\
name: chain
description: Emits a follow-up for itself.
initial: work
states:
  work:
    invoke: work
    emits: [chain]
    transitions: { done: done, error: failed }
  done: { final: true }
  failed: { final: true }
";

const NEST: &str = "\
name: nest
description: Emits a follow-up for itself until depth 2, then runs leaf as a child.
initial: work
states:
  work:
    invoke: work
    emits: [nest]
    transitions: { done: call, error: failed }
  call:
    invoke: { machine: leaf }
    transitions: { done: done, error: failed }
  done: { final: true }
  failed: { final: true }
";

const LEAF: &str = "\
name: leaf
description: Finishes at once.
initial: done
states:
  done: { final: true }
  failed: { final: true }
";

/// Logs `<depth> <DECREE_MAX_DEPTH>` to `depths.log`, then emits a follow-up for its own
/// machine (for `nest`, only below depth 2), its `decree emit` errors going to `emit.log`.
const WORK: &str = r#"#!/usr/bin/env bash
depth=$(sed -n 's/^depth: //p' "$DECREE_MESSAGE")
depth=${depth:-0}
echo "$depth $DECREE_MAX_DEPTH" >> "$DECREE_PROJECT_ROOT/depths.log"
if [ "$DECREE_MACHINE" = nest ] && [ "$depth" -ge 2 ]; then exit 0; fi
echo next | "$DECREE" emit --machine "$DECREE_MACHINE" 2>> "$DECREE_PROJECT_ROOT/emit.log"
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
        fs::write(decree.join("machines/chain.yml"), CHAIN).unwrap();
        fs::write(decree.join("machines/nest.yml"), NEST).unwrap();
        fs::write(decree.join("machines/leaf.yml"), LEAF).unwrap();
        write_script(&decree.join("scripts/work"), WORK);
        Project { tmp }
    }

    fn decree(&self) -> PathBuf {
        self.tmp.path().join(".decree")
    }

    fn write(&self, rel: &str, text: &str) {
        fs::write(self.decree().join(rel), text).unwrap();
    }

    fn queue(&self, machine: &str) {
        self.write(
            "inbox/start.md",
            &format!("---\nmachine: {machine}\n---\nstart\n"),
        );
    }

    fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.tmp.path().join(rel)).unwrap_or_default()
    }

    /// The depths the scripts ran at, each with the `DECREE_MAX_DEPTH` it got.
    fn depths(&self) -> Vec<String> {
        self.read("depths.log").lines().map(String::from).collect()
    }

    /// Every run's `message.md`.
    fn messages(&self) -> Vec<String> {
        fs::read_dir(self.decree().join("runs"))
            .unwrap()
            .map(|e| fs::read_to_string(e.unwrap().path().join("message.md")).unwrap())
            .collect()
    }

    /// `decree <args>` with no `DECREE_*` or trace variables but `env`, so a test suite run
    /// inside a decree run (a gate script) sees the same results.
    fn decree_cmd(&self, env: &[(&str, &str)], args: &[&str]) -> Command {
        let mut cmd = cargo_bin_cmd!("decree");
        cmd.current_dir(self.tmp.path()).env("NO_COLOR", "1");
        for (name, _) in std::env::vars_os() {
            let name = name.to_string_lossy().into_owned();
            if name.starts_with("DECREE_") || name.starts_with("TRACE") {
                cmd.env_remove(name);
            }
        }
        cmd.envs(env.iter().copied()).args(args);
        cmd
    }
}

fn stderr(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

const REFUSED: &str = "(set DECREE_MAX_DEPTH to change it)";

/// AC: without `DECREE_MAX_DEPTH` a chain stops at depth 10 as before, and the refused emit
/// names the variable.
#[test]
fn without_the_variable_a_chain_stops_at_depth_10_naming_it() {
    let p = Project::new();
    p.queue("chain");
    let output = p.decree_cmd(&[], &["process"]).output().unwrap();
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    let depths = p.depths();
    assert_eq!(depths.len(), 11, "{depths:?}");
    assert_eq!(depths.last().unwrap(), "10 10");
    let emit = p.read("emit.log");
    assert!(
        emit.contains(&format!("depth 11 exceeds max_depth 10 {REFUSED}")),
        "{emit}"
    );
}

/// AC: `DECREE_MAX_DEPTH=20` in `.decree/.env` lets a chain run 15 deep and further, and
/// refuses the emit at depth 21. Every script gets the limit, so its `decree emit` uses it.
#[test]
fn dotenv_raises_max_depth_for_process_and_the_emits_in_its_runs() {
    let p = Project::new();
    p.write(".env", "DECREE_MAX_DEPTH=20\n");
    p.queue("chain");
    let output = p.decree_cmd(&[], &["process"]).output().unwrap();
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    let depths = p.depths();
    let want: Vec<String> = (0..=20).map(|d| format!("{d} 20")).collect();
    assert_eq!(depths, want);
    assert!(p
        .messages()
        .iter()
        .any(|m| m.contains("depth: 15\n") && m.contains("state: done\n")));
    let emit = p.read("emit.log");
    assert_eq!(emit.lines().count(), 1, "{emit}");
    assert!(
        emit.contains(&format!("depth 21 exceeds max_depth 20 {REFUSED}")),
        "{emit}"
    );
}

/// AC: `DECREE_MAX_DEPTH=2` in the shell beats `20` in `.decree/.env`: the run at depth 2
/// starts no `machine:` child, and its event is `error` with `max_depth 2 reached`.
#[test]
fn the_shell_beats_dotenv_and_stops_a_child_run() {
    let p = Project::new();
    p.write(".env", "DECREE_MAX_DEPTH=20\n");
    p.queue("nest");
    let output = p
        .decree_cmd(&[("DECREE_MAX_DEPTH", "2")], &["process"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert_eq!(p.depths(), ["0 2", "1 2", "2 2"]);
    let messages = p.messages();
    // Children of the runs at depth 0 and 1, none of the one at depth 2.
    let leaves: Vec<&String> = messages
        .iter()
        .filter(|m| m.contains("machine: leaf\n"))
        .collect();
    assert_eq!(leaves.len(), 2, "{messages:?}");
    assert!(!messages.iter().any(|m| m.contains("depth: 3\n")));
    let deepest = fs::read_dir(p.decree().join("runs"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|dir| {
            let m = fs::read_to_string(dir.join("message.md")).unwrap();
            m.contains("machine: nest\n") && m.contains("depth: 2\n")
        })
        .unwrap();
    let events = fs::read_to_string(deepest.join("events.jsonl")).unwrap();
    let refused = events
        .lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
        .find(|e| e["type"] == "transition" && e["from"] == "call")
        .unwrap();
    assert_eq!(refused["event"], "error");
    assert_eq!(refused["error"], "max_depth 2 reached");
}

/// AC: a value that is not a whole number of at least 1, in the shell or `.decree/.env`,
/// makes `check`, `process`, `daemon` and `emit` exit 1 naming the variable and the value, and
/// `process` and `daemon` run nothing.
#[test]
fn a_bad_value_is_refused_before_anything_runs() {
    for value in ["0", "ten"] {
        for in_dotenv in [false, true] {
            let p = Project::new();
            p.queue("chain");
            let env: Vec<(&str, &str)> = if in_dotenv {
                p.write(".env", &format!("DECREE_MAX_DEPTH={value}\n"));
                Vec::new()
            } else {
                vec![("DECREE_MAX_DEPTH", value)]
            };
            let named_in = |place: &str| format!("DECREE_MAX_DEPTH in {place} is `{value}`");
            let commands = [
                &["check"][..],
                &["process"],
                &["daemon"],
                &["emit", "--machine", "chain"],
            ];
            for args in commands {
                let output = p
                    .decree_cmd(&env, args)
                    .write_stdin("x\n")
                    .timeout(std::time::Duration::from_secs(30))
                    .output()
                    .unwrap();
                assert_eq!(output.status.code(), Some(1), "{args:?} {value}");
                let out = format!(
                    "{}{}",
                    String::from_utf8_lossy(&output.stdout),
                    stderr(&output)
                );
                let want = match (args[0], in_dotenv) {
                    ("check", true) => format!(
                        ".env: line 1: `DECREE_MAX_DEPTH` is `{value}`: it must be a whole \
                         number of at least 1 (E1)"
                    ),
                    (_, true) => named_in(".decree/.env"),
                    (_, false) => named_in("the environment"),
                };
                assert!(out.contains(&want), "{args:?} {value} {in_dotenv}: {out}");
            }
            assert!(p.depths().is_empty(), "process and daemon ran nothing");
            assert_eq!(
                fs::read_dir(p.decree().join("runs")).unwrap().count(),
                0,
                "no run folder"
            );
            let inbox: Vec<_> = fs::read_dir(p.decree().join("inbox"))
                .unwrap()
                .map(|e| e.unwrap().file_name())
                .collect();
            assert_eq!(
                inbox,
                ["start.md"],
                "the message stays queued, emit queued none"
            );
        }
    }
}

/// AC: a machine's `env_file` may not set `DECREE_MAX_DEPTH`, as no other `DECREE_*` name.
#[test]
fn a_machine_env_file_may_not_set_it() {
    let p = Project::new();
    p.write(
        "machines/chain.yml",
        &CHAIN.replace("initial: work\n", "initial: work\nenv_file: .env.chain\n"),
    );
    p.write(".gitignore", ".env*\n");
    for (key, message) in [
        (
            "DECREE_MAX_DEPTH",
            "key `DECREE_MAX_DEPTH` is reserved: only `.decree/.env` sets it",
        ),
        (
            "DECREE_FOO",
            "key `DECREE_FOO` is reserved: `DECREE_*`, `TRACEPARENT` and `TRACESTATE` belong to decree",
        ),
    ] {
        p.write(".env.chain", &format!("{key}=20\n"));
        let output = p.decree_cmd(&[], &["check"]).output().unwrap();
        assert_eq!(output.status.code(), Some(1), "{key}");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains(&format!(".env.chain: line 1: {message} (E1)")),
            "{key}: {stdout}"
        );
    }
    // In `.decree/.env` it is a setting, and check passes.
    fs::remove_file(p.decree().join(".env.chain")).unwrap();
    p.write("machines/chain.yml", CHAIN);
    p.write(".env", "DECREE_MAX_DEPTH=20\n");
    let output = p.decree_cmd(&[], &["check"]).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}
