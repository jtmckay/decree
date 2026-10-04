//! The built-in `develop` and `rust_develop` machines `decree init` writes (docs/reference/cli.md,
//! `decree init`): they pass `decree check`, end in the same outcome as
//! 0.4.2's routines on the same message, and their scripts wait out Claude's usage
//! limit and resume the session. `claude`, `cargo`, `date` and `sleep` are stubs on
//! `PATH`; no test calls a model.

use assert_cmd::cargo::cargo_bin_cmd;
use assert_cmd::Command;
use serde_json::Value;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const MESSAGE: &str =
    "# Add a greeting\n\nPrint hello.\n\n## Acceptance Criteria\n\n- It prints hello.\n";

/// Records its flags and the prompt's first line in `calls`. With `CLAUDE_LIMIT` set,
/// the first call prints it and exits 1; a prompt containing `CLAUDE_FAIL_ON` exits 1.
const STUB_CLAUDE: &str = r#"#!/usr/bin/env bash
dir=$(dirname "$0")
prompt=${!#}
printf '%s | %s\n' "${*:1:$#-1}" "$(head -n 1 <<<"$prompt")" >> "$dir/calls"
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

/// Exits `CARGO_EXIT` (default 0).
const STUB_CARGO: &str = "#!/usr/bin/env bash\necho \"cargo $*\"\nexit \"${CARGO_EXIT:-0}\"\n";

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
        let path = self.bin().join(name);
        fs::write(&path, text).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
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

    /// Run 0.4.2's routine (filled for claude, as its `init` wrote it) the way 0.4.2
    /// ran it, `bash <script>` with its variables: `done` if it exits 0, else `failed`
    /// (0.4.2 re-ran it on failure, which ends the same way with these stubs).
    fn run_0_4_2(&self, routine: &str, env: &[(&str, &str)]) -> String {
        let script = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/scripts/v0_4_2")
            .join(format!("{routine}.sh"));
        let run_dir = self.root().join("run-0.4.2");
        fs::create_dir_all(&run_dir).unwrap();
        let message = run_dir.join("message.md");
        fs::write(&message, MESSAGE).unwrap();
        let mut cmd = std::process::Command::new("bash");
        cmd.arg(&script)
            .current_dir(self.root())
            .env("PATH", self.path())
            .env("message_file", &message)
            .env("message_dir", &run_dir)
            .env("message_id", "D0001-1200-01-add-greeting-0")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        for (k, v) in env {
            cmd.env(k, v);
        }
        let status = cmd.status().unwrap();
        if status.success() { "done" } else { "failed" }.to_string()
    }
}

/// AC: each ported machine passes `decree check` (with every backend `init` supports).
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

/// AC: on the same message with the same stub `claude` (and `cargo`), 0.4.2's routine
/// and the ported machine end in the same outcome.
#[test]
fn develop_and_rust_develop_end_like_0_4_2() {
    let cases: &[(&str, &str, Env, &str)] = &[
        ("develop", "develop", &[], "done"),
        (
            "develop",
            "develop",
            &[("CLAUDE_FAIL_ON", "Read")],
            "failed",
        ),
        (
            "develop",
            "develop",
            &[("CLAUDE_FAIL_ON", "Verify that")],
            "failed",
        ),
        ("rust-develop", "rust_develop", &[], "done"),
        // 0.4.2 ignored a failed build and test; qa sees them.
        (
            "rust-develop",
            "rust_develop",
            &[("CARGO_EXIT", "101")],
            "done",
        ),
        (
            "rust-develop",
            "rust_develop",
            &[("CLAUDE_FAIL_ON", "senior Rust engineer")],
            "failed",
        ),
        (
            "rust-develop",
            "rust_develop",
            &[("CLAUDE_FAIL_ON", "Fix any failures")],
            "failed",
        ),
    ];
    for (routine, machine, env, want) in cases {
        let p = Project::init();
        let old = p.run_0_4_2(routine, env);
        let id = p.run(machine, env);
        let new = p.outcome(&id);
        assert_eq!(old, *want, "0.4.2 {routine} {env:?}");
        assert_eq!(new, *want, "{machine} {env:?}: {:?}", p.events(&id));
    }
}

/// Every state `rust_develop` passes through when the build and tests fail: the
/// failures go to `qa`, which reads the logs `build` and `test` kept.
#[test]
fn rust_develop_hands_build_and_test_failures_to_qa() {
    let p = Project::init();
    let id = p.run("rust_develop", &[("CARGO_EXIT", "101")]);
    assert_eq!(p.outcome(&id), "done");
    let path: Vec<String> = p
        .events(&id)
        .iter()
        .filter(|e| e["type"] == "transition")
        .map(|e| format!("{} -{}-> {}", e["from"], e["event"], e["to"]).replace('"', ""))
        .collect();
    assert_eq!(
        path,
        [
            "null -claimed-> precheck",
            "precheck -done-> implement",
            "implement -done-> build",
            "build -error-> test",
            "test -error-> qa",
            "qa -done-> done",
        ]
    );
    let run = p.run_dir(&id);
    assert!(fs::read_to_string(run.join("build.log"))
        .unwrap()
        .contains("cargo build --release"));
    assert!(fs::read_to_string(run.join("test-output.log"))
        .unwrap()
        .contains("cargo test"));
    let qa = p.calls().pop().unwrap();
    assert!(
        qa.ends_with(&format!(
            "| Read {}/message.md, build output at {}/build.log,",
            run.display(),
            run.display()
        )),
        "{qa}"
    );
}

/// The session flags of a call line: `-p --session-id <id>` or `-p --resume <id>`.
fn session(call: &str) -> (&str, &str) {
    let flags: Vec<&str> = call.split(" | ").next().unwrap().split(' ').collect();
    assert_eq!(flags.len(), 3, "{call}");
    assert_eq!(flags[0], "-p", "{call}");
    (flags[1], flags[2])
}

/// AC: a stub `claude` prints a usage-limit message with a reset time; the script
/// waits until the reset, then resumes the same session. Also 0.4.2's fallbacks: a
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
            // The next AI step starts a session of its own.
            let (flag, next) = session(calls.last().unwrap());
            assert_eq!(flag, "--session-id", "{calls:?}");
            assert_ne!(next, first);

            // One implement attempt: the wait is inside the script, not a decree retry.
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
/// a new session each, as 0.4.2's retries did.
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
    assert_eq!(calls.len(), 3, "max_attempts 3: {calls:?}");
    let sessions: Vec<&str> = calls.iter().map(|c| session(c).1).collect();
    assert!(calls.iter().all(|c| session(c).0 == "--session-id"));
    assert!(sessions[0] != sessions[1] && sessions[1] != sessions[2]);
}
