//! The built-in `develop` and `rust_develop` machines `decree init` writes (docs/reference/cli.md,
//! `decree init`): they pass `decree check`, `develop` ends in `done` or `failed` as its
//! steps succeed or fail, `rust_develop` runs QA only when its gate
//! fails and stops on a `STOP` file, and their scripts wait out Claude's usage
//! limit and resume the session. `claude`, `cargo`, `date` and `sleep` are stubs on
//! `PATH`; no test calls a model.

use assert_cmd::cargo::cargo_bin_cmd;
use assert_cmd::Command;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

mod common;
use common::write_script;

const MESSAGE: &str =
    "# Add a greeting\n\nPrint hello.\n\n## Acceptance Criteria\n\n- It prints hello.\n";

/// Records its flags and the prompt's first line in `calls`. With `CLAUDE_LIMIT` set,
/// the first call prints it and exits 1; a prompt containing `CLAUDE_FAIL_ON` exits 1.
/// With `CLAUDE_STOP` set, it writes that to the run's `STOP` file, as an agent that
/// cannot go on would.
const STUB_CLAUDE: &str = r#"#!/usr/bin/env bash
dir=$(dirname "$0")
prompt=${!#}
printf '%s | %s\n' "${*:1:$#-1}" "$(head -n 1 <<<"$prompt")" >> "$dir/calls"
if [ -n "${CLAUDE_STOP:-}" ]; then
  echo "$CLAUDE_STOP" > "$DECREE_RUN_DIR/STOP"
fi
if [ -n "${CLAUDE_LIMIT:-}" ] && [ ! -e "$dir/limited" ]; then
  touch "$dir/limited"
  echo "$CLAUDE_LIMIT"
  exit 1
fi
if [ -n "${CLAUDE_FAIL_ON:-}" ] && [[ $prompt == *"$CLAUDE_FAIL_ON"* ]]; then
  echo "stub failure" >&2
  exit 1
fi
echo "stub done"
"#;

/// Exits `CARGO_EXIT` (default 0). With `CARGO_FAIL_ONCE` set, only its first call
/// fails, as if QA then fixed the code.
const STUB_CARGO: &str = r#"#!/usr/bin/env bash
echo "cargo $*"
if [ -n "${CARGO_FAIL_ONCE:-}" ] && [ ! -e "$(dirname "$0")/failed" ]; then
  touch "$(dirname "$0")/failed"
  exit 101
fi
exit "${CARGO_EXIT:-0}"
"#;

/// The local time is always `STUB_NOW`.
const STUB_DATE: &str = "#!/usr/bin/env bash\necho \"$STUB_NOW\"\n";

/// Records how long it was asked to sleep, and returns at once.
const STUB_SLEEP: &str = "#!/usr/bin/env bash\necho \"$1\" >> \"$(dirname \"$0\")/slept\"\n";

struct Project {
    tmp: TempDir,
}

impl Project {
    /// `decree init --ai claude`, plus a `bin/` holding stub `claude` and `cargo`.
    fn init() -> Project {
        let p = Project {
            tmp: TempDir::new().unwrap(),
        };
        p.decree(&["init", "--ai", "claude"]).assert().success();
        fs::create_dir(p.bin()).unwrap();
        p.stub("claude", STUB_CLAUDE);
        p.stub("cargo", STUB_CARGO);
        p
    }

    fn root(&self) -> &Path {
        self.tmp.path()
    }

    fn bin(&self) -> PathBuf {
        self.root().join("bin")
    }

    fn stub(&self, name: &str, text: &str) {
        write_script(&self.bin().join(name), text);
    }

    /// `PATH` with the stubs first.
    fn path(&self) -> String {
        format!(
            "{}:{}",
            self.bin().display(),
            std::env::var("PATH").unwrap_or_default()
        )
    }

    fn decree(&self, args: &[&str]) -> Command {
        let mut cmd = cargo_bin_cmd!("decree");
        cmd.current_dir(self.root())
            .env("NO_COLOR", "1")
            .env_remove("DECREE_MACHINE")
            .env_remove("DECREE_STATE")
            .env_remove("DECREE_MESSAGE_ID")
            .args(args);
        cmd
    }

    /// `decree emit --machine <machine>` then `decree process`, with the stubs on
    /// `PATH` and `env` set: the run's id.
    fn run(&self, machine: &str, env: &[(&str, &str)]) -> String {
        let out = self
            .decree(&["emit", "--machine", machine])
            .write_stdin(MESSAGE)
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let id = String::from_utf8(out).unwrap().trim().to_string();
        let mut cmd = self.decree(&["process"]);
        cmd.env("PATH", self.path());
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd.output().unwrap();
        id
    }

    fn run_dir(&self, id: &str) -> PathBuf {
        self.root().join(".decree/runs").join(id)
    }

    fn events(&self, id: &str) -> Vec<Value> {
        fs::read_to_string(self.run_dir(id).join("events.jsonl"))
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    /// The root final state the run finished in.
    fn outcome(&self, id: &str) -> String {
        let events = self.events(id);
        let last = events.last().unwrap();
        assert_eq!(last["type"], "run_finished", "{events:?}");
        last["state"].as_str().unwrap().to_string()
    }

    /// The stub `claude`'s calls so far, one line each: flags, then the prompt's first line.
    fn calls(&self) -> Vec<String> {
        fs::read_to_string(self.bin().join("calls"))
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect()
    }
}

/// AC: each built-in machine passes `decree check` (with every backend `init` supports).
#[test]
fn init_writes_develop_and_rust_develop_and_they_pass_check() {
    for ai in ["claude", "opencode", "copilot"] {
        let tmp = TempDir::new().unwrap();
        let p = Project { tmp };
        p.decree(&["init", "--ai", ai]).assert().success();
        for machine in ["develop", "rust_develop"] {
            assert!(
                p.root()
                    .join(format!(".decree/machines/{machine}.yml"))
                    .is_file(),
                "{ai} {machine}"
            );
            assert!(
                p.root()
                    .join(format!(".decree/graph/{machine}.md"))
                    .is_file(),
                "{ai} {machine}"
            );
        }
        p.decree(&["check"]).assert().code(0);
    }
}

/// Environment variables for the stubs.
type Env = &'static [(&'static str, &'static str)];

/// `develop` ends in `done` when every step succeeds, and in `failed` when the agent fails
/// to implement or to verify.
#[test]
fn develop_ends_done_or_failed() {
    let cases: &[(Env, &str)] = &[
        (&[], "done"),
        (&[("CLAUDE_FAIL_ON", "Read")], "failed"),
        (&[("CLAUDE_FAIL_ON", "Verify that")], "failed"),
    ];
    for (env, want) in cases {
        let p = Project::init();
        let id = p.run("develop", env);
        assert_eq!(p.outcome(&id), *want, "{env:?}: {:?}", p.events(&id));
    }
}

/// The transitions a run took, as `from -event-> to`.
fn path(p: &Project, id: &str) -> Vec<String> {
    p.events(id)
        .iter()
        .filter(|e| e["type"] == "transition")
        .map(|e| format!("{} -{}-> {}", e["from"], e["event"], e["to"]).replace('"', ""))
        .collect()
}

/// A gate that passes the first time skips QA.
#[test]
fn rust_develop_skips_qa_when_the_gate_passes() {
    let p = Project::init();
    let id = p.run("rust_develop", &[]);
    assert_eq!(p.outcome(&id), "done");
    assert_eq!(
        path(&p, &id),
        [
            "null -claimed-> precheck",
            "precheck -done-> implement",
            "implement -done-> gate",
            "gate -done-> done",
        ]
    );
    let gate = fs::read_to_string(p.run_dir(&id).join("gate.log")).unwrap();
    assert_eq!(
        gate,
        "cargo fmt --check\ncargo clippy --all-targets -- -D warnings\ncargo test\n"
    );
    let calls = p.calls();
    assert_eq!(calls.len(), 1, "implement only: {calls:?}");
    assert!(
        calls[0].contains("| You are a senior Rust engineer. Read "),
        "{calls:?}"
    );
}

/// A failed gate goes to QA, which reads gate.log; the final gate decides the outcome.
#[test]
fn rust_develop_hands_a_failed_gate_to_qa() {
    let qa_path = [
        "null -claimed-> precheck",
        "precheck -done-> implement",
        "implement -done-> gate",
        "gate -error-> qa",
        "qa -done-> final_gate",
    ];
    for (env, last, want) in [
        (("CARGO_FAIL_ONCE", "1"), "final_gate -done-> done", "done"),
        (
            ("CARGO_EXIT", "101"),
            "final_gate -error-> failed",
            "failed",
        ),
    ] {
        let p = Project::init();
        let id = p.run("rust_develop", &[env]);
        assert_eq!(p.outcome(&id), want, "{env:?}");
        let mut expected: Vec<&str> = qa_path.to_vec();
        expected.push(last);
        assert_eq!(path(&p, &id), expected, "{env:?}");
        let run = p.run_dir(&id);
        let qa = p.calls().pop().unwrap();
        assert!(
            qa.ends_with(&format!(
                "| Read {}/message.md. The gate (cargo fmt --check, cargo clippy",
                run.display()
            )),
            "{qa}"
        );
    }
}

/// An agent that writes `STOP` fails the run without retrying, and the file keeps
/// stopping it until a person deletes it.
#[test]
fn rust_develop_stops_when_the_agent_writes_stop() {
    let p = Project::init();
    let id = p.run("rust_develop", &[("CLAUDE_STOP", "Which greeting?")]);
    assert_eq!(p.outcome(&id), "failed");
    assert_eq!(
        path(&p, &id).last().unwrap(),
        "implement -stop-> failed",
        "{:?}",
        p.events(&id)
    );
    assert_eq!(p.calls().len(), 1, "no retry after a stop");
    let log = fs::read_to_string(p.run_dir(&id).join("0002-implement-implement.log")).unwrap();
    assert!(log.contains("[stderr] Which greeting?\n"), "{log}");

    // `decree process --retry` with STOP still there stops again, without asking the AI.
    let mut cmd = p.decree(&["process", "--retry", &id, "--state", "implement"]);
    cmd.env("PATH", p.path());
    let out = cmd.output().unwrap();
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let retry = p
        .events(&id)
        .into_iter()
        .filter(|e| e["source"] == "retry")
        .count();
    assert_eq!(retry, 1);
    assert_eq!(p.outcome(&id), "failed");
    assert_eq!(p.calls().len(), 1);
}

/// Each Claude session is listed in sessions.txt with its state and transcript path.
#[test]
fn claude_sessions_are_listed_in_sessions_txt() {
    let p = Project::init();
    let id = p.run("rust_develop", &[("CARGO_FAIL_ONCE", "1")]);
    let sessions = fs::read_to_string(p.run_dir(&id).join("sessions.txt")).unwrap();
    let lines: Vec<Vec<&str>> = sessions.lines().map(|l| l.split(' ').collect()).collect();
    let states: Vec<&str> = lines.iter().map(|l| l[0]).collect();
    assert_eq!(states, ["implement", "qa"], "{sessions}");
    let calls = p.calls();
    for (line, call) in lines.iter().zip(&calls) {
        assert_eq!(line[1], session(call).1, "{sessions}");
        assert!(
            line[2].contains("/.claude/projects/")
                && line[2].ends_with(&format!("/{}.jsonl", line[1])),
            "{sessions}"
        );
    }
}

/// The session flags of a call line: `-p --permission-mode auto --session-id <id>` or `... --resume <id>`.
fn session(call: &str) -> (&str, &str) {
    let flags: Vec<&str> = call.split(" | ").next().unwrap().split(' ').collect();
    assert_eq!(flags.len(), 5, "{call}");
    assert_eq!(flags[..3], ["-p", "--permission-mode", "auto"], "{call}");
    (flags[3], flags[4])
}

/// AC: a stub `claude` prints a usage-limit message with a reset time; the script
/// waits until the reset, then resumes the same session. A
/// reset time already passed today waits until tomorrow, and no time waits an hour.
#[test]
fn usage_limit_waits_until_the_reset_then_resumes_the_session() {
    for (limit, now, slept, until) in [
        (
            "Claude AI usage limit reached. Limits reset at 10:00 PM",
            "21:30:00",
            "1800",
            "22:00 (30m 0s)",
        ),
        (
            "usage limit reached\nlimits resets at 5:07 am",
            "04:00:30",
            "3990",
            "05:07 (66m 30s)",
        ),
        (
            "Claude AI usage limit reached. Limits reset at 10:00 PM",
            "23:00:00",
            "82800",
            "22:00 (1380m 0s)",
        ),
        (
            "You've hit your session limit · resets 1:10am (America/Denver)",
            "00:40:00",
            "1800",
            "01:10 (30m 0s)",
        ),
        (
            "Weekly limit reached ∙ resets 3am",
            "02:00:00",
            "3600",
            "03:00 (60m 0s)",
        ),
        (
            "USAGE LIMIT exceeded. Will RESET tomorrow.",
            "12:00:00",
            "3600",
            "13:00 (60m 0s)",
        ),
    ] {
        for machine in ["develop", "rust_develop"] {
            let p = Project::init();
            p.stub("date", STUB_DATE);
            p.stub("sleep", STUB_SLEEP);
            let id = p.run(machine, &[("CLAUDE_LIMIT", limit), ("STUB_NOW", now)]);
            assert_eq!(p.outcome(&id), "done", "{machine} {limit}");

            assert_eq!(
                fs::read_to_string(p.bin().join("slept")).unwrap(),
                format!("{slept}\n"),
                "{machine} {limit}"
            );
            let calls = p.calls();
            let (flag, first) = session(&calls[0]);
            assert_eq!(flag, "--session-id", "{calls:?}");
            assert_eq!(first.len(), 36, "a UUID: {first}");
            let (flag, resumed) = session(&calls[1]);
            assert_eq!((flag, resumed), ("--resume", first), "{calls:?}");
            assert_eq!(
                calls[0].split(" | ").nth(1),
                calls[1].split(" | ").nth(1),
                "the same prompt again: {calls:?}"
            );
            // The next AI step starts a session of its own (rust_develop's gate
            // passes, so it has no next AI step).
            if machine == "develop" {
                let (flag, next) = session(calls.last().unwrap());
                assert_eq!(flag, "--session-id", "{calls:?}");
                assert_ne!(next, first);
            }

            // One implement attempt: the wait is inside the script, not a retry of the run.
            let log =
                fs::read_to_string(p.run_dir(&id).join("0002-implement-implement.log")).unwrap();
            assert!(
                log.contains(&format!("[stderr] === claude session {first} ===\n")),
                "{log}"
            );
            assert!(log.contains(&format!("[stderr] [Claude token limit] Usage limit reached. Waiting until {until} to retry.\n")), "{log}");
            assert!(
                log.contains(&format!(
                    "[stderr] [Claude token limit] Resuming session {first}\n"
                )),
                "{log}"
            );
            let attempts = p
                .events(&id)
                .iter()
                .filter(|e| e["type"] == "script" && e["state"] == "implement")
                .count();
            assert_eq!(attempts, 1, "{machine}");
        }
    }
}

/// A failure that is not a usage limit does not wait: decree's attempts run again with
/// a new session each.
#[test]
fn other_failures_do_not_wait() {
    let p = Project::init();
    p.stub("date", STUB_DATE);
    p.stub("sleep", STUB_SLEEP);
    let id = p.run(
        "develop",
        &[("CLAUDE_FAIL_ON", "Read"), ("STUB_NOW", "12:00:00")],
    );
    assert_eq!(p.outcome(&id), "failed");
    assert!(!p.bin().join("slept").exists());
    let calls = p.calls();
    assert_eq!(calls.len(), 3, "attempts: 3: {calls:?}");
    let sessions: Vec<&str> = calls.iter().map(|c| session(c).1).collect();
    assert!(calls.iter().all(|c| session(c).0 == "--session-id"));
    assert!(sessions[0] != sessions[1] && sessions[1] != sessions[2]);
}
