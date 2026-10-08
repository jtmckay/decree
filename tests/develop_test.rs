//! The built-in `develop` machine `decree init` writes (docs/reference/cli.md, `decree init`):
//! it passes `decree check`, its scripts source `lib/ai.sh`, the default gate runs nothing
//! and says so, `verify` names `pass` or `fail` from the AI's `VERDICT:` line, a run hands a
//! failed gate to `fix` and stops on a `STOP` file, and Claude's usage limit is waited out
//! and the session resumed. `claude`, `date` and `sleep` are stubs on `PATH`, and the gate
//! is a stub script; no test calls a model.

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
/// cannot go on would. A prompt asking for a verdict is answered with `CLAUDE_REPLY`,
/// by default `VERDICT: pass`.
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
if [[ $prompt == *"VERDICT: pass"* ]]; then
  printf '%b\n' "${CLAUDE_REPLY-VERDICT: pass}"
fi
"#;

/// The gate: prints its checks to gate.log and exits `GATE_EXIT` (default 0). With
/// `GATE_FAIL_ONCE` set, only its first call fails, as if `fix` then fixed the code.
const STUB_GATE: &str = r#"#!/usr/bin/env bash
set -euo pipefail
status=${GATE_EXIT:-0}
if [ -n "${GATE_FAIL_ONCE:-}" ] && [ ! -e "$DECREE_RUN_DIR/gate.failed" ]; then
  touch "$DECREE_RUN_DIR/gate.failed"
  status=1
fi
{ echo "checks ran"; exit "$status"; } 2>&1 | tee "$DECREE_RUN_DIR/gate.log"
"#;

/// The local time is always `STUB_NOW`.
const STUB_DATE: &str = "#!/usr/bin/env bash\necho \"$STUB_NOW\"\n";

/// Records how long it was asked to sleep, and returns at once.
const STUB_SLEEP: &str = "#!/usr/bin/env bash\necho \"$1\" >> \"$(dirname \"$0\")/slept\"\n";

struct Project {
    tmp: TempDir,
}

impl Project {
    /// `decree init --ai claude`, plus a `bin/` holding a stub `claude`, and a stub gate.
    fn init() -> Project {
        let p = Project {
            tmp: TempDir::new().unwrap(),
        };
        p.decree(&["init", "--ai", "claude"]).assert().success();
        fs::create_dir(p.bin()).unwrap();
        p.stub("claude", STUB_CLAUDE);
        write_script(&p.script("gate"), STUB_GATE);
        p
    }

    /// `.decree/scripts/develop/<name>.sh`.
    fn script(&self, name: &str) -> PathBuf {
        self.root()
            .join(format!(".decree/scripts/develop/{name}.sh"))
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

    /// `decree emit --machine develop` then `decree process`, with the stubs on
    /// `PATH` and `env` set: the run's id.
    fn run(&self, env: &[(&str, &str)]) -> String {
        let out = self
            .decree(&["emit", "--machine", "develop"])
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

/// Every file under `dir`, relative to it, sorted.
fn files_under(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            out.extend(
                files_under(&path)
                    .into_iter()
                    .map(|f| format!("{name}/{f}")),
            );
        } else {
            out.push(path.file_name().unwrap().to_string_lossy().into_owned());
        }
    }
    out.sort();
    out
}

/// AC: `decree init` writes `develop` and `router` only, and `lib/ai.sh`; no script defines
/// `ai()`; everything passes `decree check` (with every backend `init` supports).
#[test]
fn init_writes_develop_router_and_lib_ai_sh_and_they_pass_check() {
    for ai in ["claude", "opencode", "copilot"] {
        let p = Project {
            tmp: TempDir::new().unwrap(),
        };
        p.decree(&["init", "--ai", ai]).assert().success();
        let decree = p.root().join(".decree");
        assert_eq!(
            files_under(&decree.join("machines")),
            ["develop.yml", "router.yml"],
            "{ai}"
        );
        assert_eq!(
            files_under(&decree.join("scripts")),
            [
                "develop/fix.sh".to_string(),
                "develop/gate.sh".to_string(),
                "develop/implement.sh".to_string(),
                "develop/precheck.sh".to_string(),
                "develop/verify.sh".to_string(),
                "git_baseline.sh".to_string(),
                "router/".to_string() + &format!("ask_{ai}.sh"),
                "snapshot.sh".to_string(),
            ],
            "{ai}"
        );
        assert_eq!(
            files_under(&decree.join("lib")),
            ["README.md", "ai.sh"],
            "{ai}"
        );
        let lib = fs::read_to_string(decree.join("lib/ai.sh")).unwrap();
        assert!(lib.contains("\nai() {\n"), "{ai}: {lib}");
        for script in files_under(&decree.join("scripts")) {
            let text = fs::read_to_string(decree.join("scripts").join(&script)).unwrap();
            assert!(!text.contains("ai()"), "{ai}: {script} defines ai()");
        }
        assert!(decree.join("graph/develop.md").is_file(), "{ai}");
        p.decree(&["check"]).assert().code(0);
    }
}

/// The gate `init` writes runs no checks, says so in its output and in gate.log, and exits 0.
#[test]
fn the_default_gate_exits_0_and_says_it_is_unconfigured() {
    let p = Project {
        tmp: TempDir::new().unwrap(),
    };
    p.decree(&["init", "--ai", "claude"]).assert().success();
    let run_dir = p.root().join("run");
    fs::create_dir(&run_dir).unwrap();
    let out = std::process::Command::new(p.script("gate"))
        .current_dir(p.root())
        .env("DECREE_RUN_DIR", &run_dir)
        .output()
        .unwrap();
    let said = "gate: no checks configured; edit .decree/scripts/develop/gate.sh\n";
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(String::from_utf8_lossy(&out.stdout), said);
    assert_eq!(fs::read_to_string(run_dir.join("gate.log")).unwrap(), said);
    let text = fs::read_to_string(p.script("gate")).unwrap();
    for check in [
        "# cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test\n",
        "# npm ci && npm run lint && npm test\n",
        "# gofmt -l . | (! grep .) && go vet ./... && go test ./...\n",
    ] {
        assert!(text.contains(check), "{check}");
    }
}

/// `verify.sh` run on its own, with the stub `claude` replying `reply`: its exit code and
/// the event it wrote.
fn verify_alone(reply: &str) -> (Option<i32>, String) {
    let p = Project::init();
    let run_dir = p.root().join("run");
    fs::create_dir(&run_dir).unwrap();
    fs::write(run_dir.join("message.md"), MESSAGE).unwrap();
    let event_file = run_dir.join("event");
    fs::write(&event_file, "").unwrap();
    let out = std::process::Command::new(p.script("verify"))
        .current_dir(p.root())
        .env("PATH", p.path())
        .env("DECREE_LIB", p.root().join(".decree/lib"))
        .env("DECREE_RUN_DIR", &run_dir)
        .env("DECREE_MESSAGE", run_dir.join("message.md"))
        .env("DECREE_EVENT_FILE", &event_file)
        .env("DECREE_STATE", "verify")
        .env("CLAUDE_REPLY", reply)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("stub done\n"), "{out:?}");
    (out.status.code(), fs::read_to_string(&event_file).unwrap())
}

/// `verify` names the last `VERDICT:` line's `pass` or `fail`, and fails without one.
#[test]
fn verify_writes_the_last_verdict_or_fails_without_one() {
    for (reply, want) in [
        ("All criteria met.\nVERDICT: pass", (Some(0), "pass\n")),
        ("A test fails.\nVERDICT: fail", (Some(0), "fail\n")),
        ("**VERDICT: fail**", (Some(0), "fail\n")),
        (
            "VERDICT: pass\nOn a second look, no.\nVERDICT: fail",
            (Some(0), "fail\n"),
        ),
        ("Everything passes.", (Some(1), "")),
        ("VERDICT: maybe", (Some(1), "")),
        ("", (Some(1), "")),
    ] {
        let (code, event) = verify_alone(reply);
        assert_eq!((code, event.as_str()), want, "{reply:?}");
    }
}

/// Environment variables for the stubs.
type Env = &'static [(&'static str, &'static str)];

/// The transitions a run took, as `from -event-> to`.
fn path(p: &Project, id: &str) -> Vec<String> {
    p.events(id)
        .iter()
        .filter(|e| e["type"] == "transition")
        .map(|e| format!("{} -{}-> {}", e["from"], e["event"], e["to"]).replace('"', ""))
        .collect()
}

/// A gate that passes the first time skips `fix`: implement, gate, verify, done.
#[test]
fn develop_skips_fix_when_the_gate_passes() {
    let p = Project::init();
    let id = p.run(&[]);
    assert_eq!(p.outcome(&id), "done");
    assert_eq!(
        path(&p, &id),
        [
            "null -claimed-> precheck",
            "precheck -done-> implement",
            "implement -done-> gate",
            "gate -done-> verify",
            "verify -pass-> done",
        ]
    );
    assert_eq!(
        fs::read_to_string(p.run_dir(&id).join("gate.log")).unwrap(),
        "checks ran\n"
    );
    let calls = p.calls();
    assert_eq!(calls.len(), 2, "implement and verify: {calls:?}");
    assert!(
        calls[0].contains("| You are a senior engineer. Read "),
        "{calls:?}"
    );
    assert!(calls[1].contains("| Read "), "{calls:?}");
}

/// A failed gate goes to `fix`, which reads gate.log; the final gate decides whether
/// verify runs.
#[test]
fn develop_hands_a_failed_gate_to_fix() {
    let fix_path = [
        "null -claimed-> precheck",
        "precheck -done-> implement",
        "implement -done-> gate",
        "gate -error-> fix",
        "fix -done-> final_gate",
    ];
    let cases: &[(Env, &[&str], &str)] = &[
        (
            &[("GATE_FAIL_ONCE", "1")],
            &["final_gate -done-> verify", "verify -pass-> done"],
            "done",
        ),
        (
            &[("GATE_EXIT", "1")],
            &["final_gate -error-> failed"],
            "failed",
        ),
    ];
    for (env, last, want) in cases {
        let p = Project::init();
        let id = p.run(env);
        assert_eq!(p.outcome(&id), *want, "{env:?}");
        let mut expected: Vec<&str> = fix_path.to_vec();
        expected.extend(last.iter());
        assert_eq!(path(&p, &id), expected, "{env:?}");
        let run = p.run_dir(&id);
        let fix = &p.calls()[1];
        assert!(
            fix.ends_with(&format!(
                "| Read {}/message.md. The project's gate",
                run.display()
            )),
            "{fix}"
        );
    }
}

/// AC: an AI whose reply ends `VERDICT: fail` makes verify's event `fail`, and the run
/// ends in `failed`. A failing agent fails the run too.
#[test]
fn develop_ends_failed_on_a_fail_verdict_or_a_failing_agent() {
    let p = Project::init();
    let id = p.run(&[("CLAUDE_REPLY", "A test fails.\nVERDICT: fail")]);
    assert_eq!(p.outcome(&id), "failed");
    assert_eq!(path(&p, &id).last().unwrap(), "verify -fail-> failed");

    let cases: &[(Env, &str)] = &[
        (&[("CLAUDE_FAIL_ON", "senior engineer")], "implement"),
        (&[("CLAUDE_FAIL_ON", "Verify that")], "verify"),
        (&[("CLAUDE_REPLY", "")], "verify"),
    ];
    for (env, state) in cases {
        let p = Project::init();
        let id = p.run(env);
        assert_eq!(p.outcome(&id), "failed", "{env:?}");
        assert_eq!(
            path(&p, &id).last().unwrap(),
            &format!("{state} -error-> failed"),
            "{env:?}"
        );
    }
}

/// An agent that writes `STOP` fails the run without retrying, and the file keeps
/// stopping it until a person deletes it.
#[test]
fn develop_stops_when_the_agent_writes_stop() {
    let p = Project::init();
    let id = p.run(&[("CLAUDE_STOP", "Which greeting?")]);
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

    // `fix` and `verify` stop on it too.
    for state in ["fix", "verify"] {
        let mut cmd = p.decree(&["process", "--retry", &id, "--state", state]);
        cmd.env("PATH", p.path());
        cmd.output().unwrap();
        assert_eq!(
            path(&p, &id).last().unwrap(),
            &format!("{state} -stop-> failed")
        );
        assert_eq!(p.calls().len(), 1, "{state}");
    }
}

/// Each Claude session is listed in sessions.txt with its state and transcript path.
#[test]
fn claude_sessions_are_listed_in_sessions_txt() {
    let p = Project::init();
    let id = p.run(&[("GATE_FAIL_ONCE", "1")]);
    let sessions = fs::read_to_string(p.run_dir(&id).join("sessions.txt")).unwrap();
    let lines: Vec<Vec<&str>> = sessions.lines().map(|l| l.split(' ').collect()).collect();
    let states: Vec<&str> = lines.iter().map(|l| l[0]).collect();
    assert_eq!(states, ["implement", "fix", "verify"], "{sessions}");
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
        let p = Project::init();
        p.stub("date", STUB_DATE);
        p.stub("sleep", STUB_SLEEP);
        let id = p.run(&[("CLAUDE_LIMIT", limit), ("STUB_NOW", now)]);
        assert_eq!(p.outcome(&id), "done", "{limit}");

        assert_eq!(
            fs::read_to_string(p.bin().join("slept")).unwrap(),
            format!("{slept}\n"),
            "{limit}"
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

        // One implement attempt: the wait is inside the script, not a retry of the run.
        let log = fs::read_to_string(p.run_dir(&id).join("0002-implement-implement.log")).unwrap();
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
        assert_eq!(attempts, 1, "{limit}");
    }
}

/// A failure that is not a usage limit does not wait: decree's attempts run again with
/// a new session each.
#[test]
fn other_failures_do_not_wait() {
    let p = Project::init();
    p.stub("date", STUB_DATE);
    p.stub("sleep", STUB_SLEEP);
    let id = p.run(&[("CLAUDE_FAIL_ON", "Read"), ("STUB_NOW", "12:00:00")]);
    assert_eq!(p.outcome(&id), "failed");
    assert!(!p.bin().join("slept").exists());
    let calls = p.calls();
    assert_eq!(calls.len(), 3, "attempts: 3: {calls:?}");
    let sessions: Vec<&str> = calls.iter().map(|c| session(c).1).collect();
    assert!(calls.iter().all(|c| session(c).0 == "--session-id"));
    assert!(sessions[0] != sessions[1] && sessions[1] != sessions[2]);
}
