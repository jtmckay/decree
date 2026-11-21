//! The built-in `develop` machine `decree init` writes (docs/reference/cli.md, `decree init`):
//! it passes `decree check`, its scripts source the `ai.sh` beside them, the default gate runs nothing
//! and says so, `verify` names `pass` or `fail` from the AI's `VERDICT:` line, a run hands a
//! failed gate to `fix`, ends a gate that could not run in `failed`, and stops on a `STOP`
//! file, and Claude's usage limit is waited out and the session resumed. `claude`, `date`,
//! `sleep` and the gate's one check are stubs on `PATH`; no test calls a model.

use assert_cmd::Command;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

mod common;
#[path = "common/schema.rs"]
mod schema;
use common::write_script;

const MESSAGE: &str =
    "# Add a greeting\n\nPrint hello.\n\n## Acceptance Criteria\n\n- It prints hello.\n";

/// Records its flags and the prompt's first line in `calls`. With `CLAUDE_LIMIT` set,
/// the first call prints it and exits 1; a prompt containing `CLAUDE_FAIL_ON` exits 1.
/// With `CLAUDE_STOP` set, it writes that to the run's `STOP` file, as an agent that
/// cannot go on would. With `CLAUDE_DECLARE` set, it appends that line to
/// `machines/infra/scripts/requirements`, as infra's `declare` asks. With `CLAUDE_DO` set, it runs that
/// bash, with the prompt in `$prompt`, as an agent that edits files would. With `CLAUDE_RUNS_GATE` set, it runs the gate, as `fix` asks. A prompt asking for a verdict is answered with `CLAUDE_REPLY`,
/// by default `VERDICT: pass`.
const STUB_CLAUDE: &str = r#"#!/usr/bin/env bash
dir=$(dirname "$0")
prompt=${!#}
printf '%s | %s\n' "${*:1:$#-1}" "$(head -n 1 <<<"$prompt")" >> "$dir/calls"
if [ -n "${CLAUDE_STOP:-}" ]; then
  echo "$CLAUDE_STOP" > "$DECREE_RUN_DIR/STOP"
fi
if [ -n "${CLAUDE_DO:-}" ]; then
  eval "$CLAUDE_DO"
fi
if [ -n "${CLAUDE_DECLARE:-}" ]; then
  echo "$CLAUDE_DECLARE" >> .decree/machines/infra/scripts/requirements
fi
if [ -n "${CLAUDE_RUNS_GATE:-}" ]; then
  .decree/machines/develop/scripts/gate.sh
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

/// The gate's one check: prints that it ran and exits `GATE_EXIT` (default 0). With
/// `GATE_FAIL_ONCE` set, only its first call fails, as if `fix` then fixed the code.
const STUB_CHECK: &str = r#"#!/usr/bin/env bash
status=${GATE_EXIT:-0}
if [ -n "${GATE_FAIL_ONCE:-}" ] && [ ! -e "$DECREE_RUN_DIR/gate.failed" ]; then
  touch "$DECREE_RUN_DIR/gate.failed"
  status=1
fi
echo "checks ran"
exit "$status"
"#;

/// The line of the gate `init` writes that stands in for the project's checks.
const UNCONFIGURED: &str =
    "  echo \"gate: no checks configured; edit .decree/machines/develop/scripts/gate.sh, then keep it with \\`decree plugin export develop\\`\"\n";

/// The local time is always `STUB_NOW`.
const STUB_DATE: &str = "#!/usr/bin/env bash\necho \"$STUB_NOW\"\n";

/// Records how long it was asked to sleep, and returns at once.
const STUB_SLEEP: &str = "#!/usr/bin/env bash\necho \"$1\" >> \"$(dirname \"$0\")/slept\"\n";

/// Who commits, for `git` and for the `commit` script decree runs.
const GIT_IDENTITY: [(&str, &str); 4] = [
    ("GIT_AUTHOR_NAME", "t"),
    ("GIT_AUTHOR_EMAIL", "t@t"),
    ("GIT_COMMITTER_NAME", "t"),
    ("GIT_COMMITTER_EMAIL", "t@t"),
];

struct Project {
    tmp: TempDir,
    /// The folder holding `.decree/`: the temp dir, or a subfolder of it.
    root: PathBuf,
}

impl Project {
    /// `decree init --ai claude` and `decree plugin add develop qa router`, plus a `bin/`
    /// holding a stub `claude`, and the gate `develop` installed with its checks replaced by
    /// the stub `check`, exported.
    fn init() -> Project {
        Project::init_in("")
    }

    /// An empty folder, before `decree init`.
    fn empty() -> Project {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().to_path_buf();
        Project { tmp, root }
    }

    /// `init`, with the project in `sub`, a folder of a larger git repository (the temp
    /// dir); "" puts it at the repository's root.
    fn init_in(sub: &str) -> Project {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join(sub);
        fs::create_dir_all(&root).unwrap();
        let p = Project { tmp, root };
        p.decree(&["init", "--ai", "claude"]).assert().success();
        p.decree(&["plugin", "add", "develop", "qa", "router"])
            .assert()
            .success();
        fs::create_dir(p.bin()).unwrap();
        p.stub("claude", STUB_CLAUDE);
        p.stub("check", STUB_CHECK);
        p.edit_gate(UNCONFIGURED, "  check\n");
        // The project's own gate is kept in its plugin file, as a person keeps it.
        p.decree(&["plugin", "export", "develop"])
            .assert()
            .success();
        fs::write(p.root().join(".gitignore"), "bin/\n").unwrap();
        p.git(&["init", "-q", p.tmp.path().to_str().unwrap()]);
        p.commit_all("init");
        p
    }

    /// `git` in the project, which must succeed: its output.
    fn git(&self, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(self.root())
            .envs(GIT_IDENTITY)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
        String::from_utf8(out.stdout).unwrap()
    }

    /// Commit every change, as a person would before queuing work, if there is one.
    fn commit_all(&self, title: &str) {
        self.git(&["add", "-A"]);
        if !self.git(&["status", "--porcelain"]).is_empty() {
            self.git(&["commit", "-qm", title]);
        }
    }

    /// The commit titles, newest first.
    fn commits(&self) -> Vec<String> {
        self.git(&["log", "--format=%s"])
            .lines()
            .map(String::from)
            .collect()
    }

    /// Replaces `from` in the gate with `to`.
    fn edit_gate(&self, from: &str, to: &str) {
        let gate = fs::read_to_string(self.script("gate")).unwrap();
        assert!(gate.contains(from), "{from}");
        write_script(&self.script("gate"), &gate.replacen(from, to, 1));
    }

    /// `.decree/machines/develop/scripts/<name>.sh`.
    fn script(&self, name: &str) -> PathBuf {
        self.root()
            .join(format!(".decree/machines/develop/scripts/{name}.sh"))
    }

    fn root(&self) -> &Path {
        &self.root
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
        let mut cmd = common::decree_at(self.root());
        cmd.env("NO_COLOR", "1")
            .env_remove("DECREE_MACHINE")
            .env_remove("DECREE_STATE")
            .env_remove("DECREE_MESSAGE_ID")
            .envs(GIT_IDENTITY)
            .args(args);
        cmd
    }

    /// `decree emit --machine develop` then `decree process`, with the stubs on
    /// `PATH` and `env` set: the run's id.
    fn run(&self, env: &[(&str, &str)]) -> String {
        self.run_on("develop", env)
    }

    /// `run`, on `machine`, after committing what the test changed (as a person would).
    fn run_on(&self, machine: &str, env: &[(&str, &str)]) -> String {
        self.commit_all("test setup");
        self.emit_and_process(machine, env)
    }

    /// `run`, leaving uncommitted changes where they are.
    fn run_on_dirty(&self, env: &[(&str, &str)]) -> String {
        self.emit_and_process("develop", env)
    }

    /// `decree emit --machine develop`, nothing processed: the message's id.
    fn emit_only(&self) -> String {
        let out = self
            .decree(&["emit", "--machine", "develop"])
            .write_stdin(MESSAGE)
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        String::from_utf8(out).unwrap().trim().to_string()
    }

    fn emit_and_process(&self, machine: &str, env: &[(&str, &str)]) -> String {
        let out = self
            .decree(&["emit", "--machine", machine])
            .write_stdin(MESSAGE)
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let id = String::from_utf8(out).unwrap().trim().to_string();
        let mut cmd = self.decree(&["daemon", "--once"]);
        cmd.env("PATH", self.path());
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd.output().unwrap();
        id
    }

    /// A `develop` run whose agent writes `hello.txt` and whose verify fails: its id and
    /// the output of the `process` that ran it.
    fn fail_with_changes(&self) -> (String, std::process::Output) {
        self.commit_all("test setup");
        let id = self.emit_only();
        let out = self
            .decree(&["daemon", "--once"])
            .env("PATH", self.path())
            .env(
                "CLAUDE_DO",
                "case \"$prompt\" in *senior*) echo hi > hello.txt ;; esac",
            )
            .env("CLAUDE_REPLY", "VERDICT: fail")
            .output()
            .unwrap();
        (id, out)
    }

    /// `.decree/attention/<id>.md`.
    fn attention(&self, id: &str) -> PathBuf {
        self.root()
            .join(".decree/attention")
            .join(format!("{id}.md"))
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

    /// The output of the run's last script in `state`.
    fn log(&self, id: &str, state: &str) -> String {
        let events = self.events(id);
        let event = events
            .iter()
            .rev()
            .find(|e| e["type"] == "script" && e["state"] == state)
            .unwrap();
        fs::read_to_string(self.run_dir(id).join(event["log"].as_str().unwrap())).unwrap()
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

/// AC (A10): `decree init` installs only its default plugins (D86); `decree plugin add develop qa router` adds
/// `develop`, `infra` (which `develop` needs), `qa` and `router`, each with its helpers, config
/// and data beside its scripts (`ai.sh` for each that asks AI, the git scripts, infra's declared
/// environment, qa's settings), and no `lib/`; no script but `ai.sh` defines `ai()`; everything
/// passes `decree check`, whichever backend `init` was given, since no file names it.
#[test]
fn plugin_add_writes_develop_infra_qa_router_and_they_pass_check() {
    let mut installed = Vec::new();
    for ai in ["claude", "opencode", "copilot"] {
        let p = Project::empty();
        p.decree(&["init", "--ai", ai]).assert().success();
        let decree = p.root().join(".decree");
        assert_eq!(
            files_under(&decree.join("plugins")),
            ["ai.md", "order.md", "prune.md", "skill.md"]
        );
        common::drop_default_plugins(p.root());
        assert_eq!(
            files_under(&decree.join("machines")),
            Vec::<String>::new(),
            "{ai}"
        );
        assert!(!decree.join("scripts").exists(), "{ai}");
        p.decree(&["plugin", "add", "develop", "qa", "router"])
            .assert()
            .success();
        assert_eq!(
            files_under(&decree.join("plugins")),
            ["ai.md", "develop.md", "infra.md", "qa.md", "router.md"],
            "{ai}"
        );
        let machines: Vec<String> = files_under(&decree.join("machines"))
            .into_iter()
            .filter(|f| f.ends_with("/machine.yml"))
            .collect();
        assert_eq!(
            machines,
            [
                "develop/machine.yml",
                "infra/machine.yml",
                "qa/machine.yml",
                "router/machine.yml"
            ],
            "{ai}"
        );
        let scripts: Vec<String> = files_under(&decree.join("machines"))
            .into_iter()
            .filter_map(|f| f.split_once("/scripts/").map(|(m, f)| format!("{m}/{f}")))
            .collect();
        assert_eq!(
            scripts,
            [
                "ai/ai.sh",
                "develop/clean_tree.sh",
                "develop/commit.sh",
                "develop/fix.sh",
                "develop/gate.sh",
                "develop/implement.sh",
                "develop/keep.sh",
                "develop/precheck.sh",
                "develop/verify.sh",
                "infra/apply.sh",
                "infra/clean_tree.sh",
                "infra/commit.sh",
                "infra/declare.sh",
                "infra/keep.sh",
                "infra/kinds/command.sh",
                "infra/requirements",
                "infra/run.sh",
                "qa/attack.sh",
                "qa/clean_tree.sh",
                "qa/commit.sh",
                "qa/invariants.md",
                "qa/keep.sh",
                "qa/qa.sh",
                "qa/recon.sh",
                "qa/report.sh",
                "qa/run.sh",
                "qa/triage.sh",
                "qa/wrenches.md",
                "router/ask.sh",
            ],
            "{ai}"
        );
        assert!(!decree.join("lib").exists(), "{ai}");
        let helper = fs::read_to_string(decree.join("machines/ai/scripts/ai.sh")).unwrap();
        assert!(helper.contains("\nai() {\n"), "{ai}");
        let mut texts = Vec::new();
        for script in &scripts {
            let (m, f) = script.split_once('/').unwrap();
            let text = fs::read_to_string(decree.join("machines").join(m).join("scripts").join(f))
                .unwrap();
            if !script.ends_with("/ai.sh") {
                assert!(!text.contains("ai()"), "{ai}: {script} defines ai()");
            }
            texts.push(text);
        }
        installed.push(texts);
        assert!(decree.join("machines/develop/graph.md").is_file(), "{ai}");
        let develop: serde_norway::Value = serde_norway::from_str(
            &fs::read_to_string(decree.join("machines/develop/machine.yml")).unwrap(),
        )
        .unwrap();
        assert_eq!(develop["states"]["gate"]["transitions"]["fail"], "fix");
        assert_eq!(
            develop["states"]["final_gate"]["transitions"]["fail"],
            "failed"
        );
        let gate = fs::read_to_string(decree.join("machines/develop/scripts/gate.sh")).unwrap();
        for event in ["echo pass > ", "echo fail > "] {
            assert!(gate.contains(event), "{ai}: {event}");
        }
        p.decree(&["check"]).assert().code(0);
    }
    assert!(
        installed.windows(2).all(|w| w[0] == w[1]),
        "the backend is in no file"
    );
}

/// `gate.sh` run on its own: its exit code, stdout, stderr, gate.log and the event it wrote.
fn gate_alone(p: &Project) -> (Option<i32>, String, String, String, String) {
    let run_dir = p.root().join("run");
    fs::create_dir_all(&run_dir).unwrap();
    let event_file = run_dir.join("event");
    fs::write(&event_file, "").unwrap();
    let out = std::process::Command::new(p.script("gate"))
        .current_dir(p.root())
        .env("PATH", p.path())
        .env("DECREE_RUN_DIR", &run_dir)
        .env("DECREE_ROOT_RUN_DIR", &run_dir)
        .env("DECREE_EVENT_FILE", &event_file)
        .output()
        .unwrap();
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        fs::read_to_string(run_dir.join("gate.log")).unwrap(),
        fs::read_to_string(&event_file).unwrap(),
    )
}

/// The gate `develop` installs runs no checks, says so in its output and in gate.log, and
/// names `pass`. Its commented checks each have a `command -v` line.
#[test]
fn the_default_gate_says_it_is_unconfigured_and_names_pass() {
    let p = Project::empty();
    p.decree(&["init", "--ai", "claude"]).assert().success();
    p.decree(&["plugin", "add", "develop"]).assert().success();
    let said = "gate: no checks configured; edit .decree/machines/develop/scripts/gate.sh, then keep it with `decree plugin export develop`\n";
    assert_eq!(
        gate_alone(&p),
        (
            Some(0),
            said.to_string(),
            String::new(),
            said.to_string(),
            "pass\n".to_string()
        )
    );
    let text = fs::read_to_string(p.script("gate")).unwrap();
    for check in [
        "# command -v cargo >/dev/null || missing cargo\n",
        "# cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test\n",
        "# command -v npm >/dev/null || missing npm\n",
        "# npm ci && npm run lint && npm test\n",
        "# command -v go >/dev/null || missing go\n",
        "# gofmt -l . | (! grep .) && go vet ./... && go test ./...\n",
    ] {
        assert!(text.contains(check), "{check}");
    }
}

/// The gate names `pass` or `fail` from its checks and exits 0; a missing tool is a
/// non-zero exit, before any check runs, that says which tool.
#[test]
fn the_gate_names_pass_or_fail_and_exits_1_when_a_tool_is_missing() {
    let p = Project::init();
    let ran = "checks ran\n".to_string();
    assert_eq!(
        gate_alone(&p),
        (
            Some(0),
            ran.clone(),
            String::new(),
            ran.clone(),
            "pass\n".into()
        )
    );
    p.edit_gate("  check\n", "  check && false\n");
    assert_eq!(
        gate_alone(&p),
        (Some(0), ran.clone(), String::new(), ran, "fail\n".into())
    );
    p.edit_gate(
        "# command -v cargo >/dev/null || missing cargo\n",
        "command -v no-such-tool >/dev/null || missing no-such-tool\n",
    );
    let said = "gate: no-such-tool not found on PATH; the checks cannot run\n".to_string();
    assert_eq!(
        gate_alone(&p),
        (Some(1), String::new(), said.clone(), said, String::new())
    );
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
        .env("DECREE_RUN_DIR", &run_dir)
        .env("DECREE_ROOT_RUN_DIR", &run_dir)
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

/// `verify` keeps gaps.md: a `fail` writes the new one, a `pass` removes it, and an attempt
/// that ends without a verdict puts the last one back.
#[test]
fn verify_leaves_gaps_on_fail_and_keeps_the_last_through_a_dead_attempt() {
    let write_gaps = "echo \"- the greeting is missing\" > \"$DECREE_ROOT_RUN_DIR/gaps.md\"";
    for (before, reply, agent, after) in [
        (
            None,
            "VERDICT: fail",
            write_gaps,
            Some("- the greeting is missing\n"),
        ),
        (
            Some("- old gap\n"),
            "VERDICT: fail",
            write_gaps,
            Some("- the greeting is missing\n"),
        ),
        (Some("- old gap\n"), "VERDICT: pass", "true", None),
        (Some("- old gap\n"), "", "true", Some("- old gap\n")),
    ] {
        let p = Project::init();
        let run_dir = p.root().join("run");
        fs::create_dir(&run_dir).unwrap();
        fs::write(run_dir.join("message.md"), MESSAGE).unwrap();
        let gaps = run_dir.join("gaps.md");
        if let Some(text) = before {
            fs::write(&gaps, text).unwrap();
        }
        let event_file = run_dir.join("event");
        fs::write(&event_file, "").unwrap();
        let out = std::process::Command::new(p.script("verify"))
            .current_dir(p.root())
            .env("PATH", p.path())
            .env("DECREE_RUN_DIR", &run_dir)
            .env("DECREE_ROOT_RUN_DIR", &run_dir)
            .env("DECREE_MESSAGE", run_dir.join("message.md"))
            .env("DECREE_EVENT_FILE", &event_file)
            .env("DECREE_STATE", "verify")
            .env("CLAUDE_REPLY", reply)
            .env("CLAUDE_DO", agent)
            .output()
            .unwrap();
        assert_eq!(
            fs::read_to_string(&gaps).ok().as_deref(),
            after,
            "{before:?} {reply:?}: {out:?}"
        );
        assert!(!run_dir.join("gaps.md.last").exists());
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
            "gate -pass-> verify",
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
        "gate -fail-> fix",
        "fix -done-> final_gate",
    ];
    let cases: &[(Env, &[&str], &str)] = &[
        (
            &[("GATE_FAIL_ONCE", "1")],
            &["final_gate -pass-> verify", "verify -pass-> done"],
            "done",
        ),
        (
            &[("GATE_EXIT", "1")],
            &["final_gate -fail-> failed"],
            "failed",
        ),
        // The agent runs the gate, as `fix` asks; its event is not implement's or fix's.
        (
            &[("GATE_EXIT", "1"), ("CLAUDE_RUNS_GATE", "1")],
            &["final_gate -fail-> failed"],
            "failed",
        ),
        // The agent finds the step's own event file and writes to it, as one did in the beta:
        // `ai` empties it after the call, so the step's event is still its own.
        (
            &[
                ("GATE_FAIL_ONCE", "1"),
                ("CLAUDE_DO", "echo pass > \"$DECREE_RUN_DIR/.event\""),
            ],
            &["final_gate -pass-> verify", "verify -pass-> done"],
            "done",
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

/// AC: a gate whose check needs a tool not on `PATH` names `error`, and the run ends in
/// `failed` without `fix`.
#[test]
fn develop_ends_failed_when_the_gate_cannot_run() {
    let p = Project::init();
    p.edit_gate(
        "# command -v cargo >/dev/null || missing cargo\n",
        "command -v no-such-tool >/dev/null || missing no-such-tool\n",
    );
    let id = p.run(&[]);
    assert_eq!(p.outcome(&id), "failed");
    assert_eq!(
        path(&p, &id),
        [
            "null -claimed-> precheck",
            "precheck -done-> implement",
            "implement -done-> gate",
            "gate -error-> failed",
        ]
    );
    assert_eq!(p.calls().len(), 1, "implement only: {:?}", p.calls());
    assert_eq!(
        fs::read_to_string(p.run_dir(&id).join("gate.log")).unwrap(),
        "gate: no-such-tool not found on PATH; the checks cannot run\n"
    );
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
    let log = p.log(&id, "implement");
    assert!(log.contains("Which greeting?\n"), "{log}");

    // `decree daemon --once --retry` with STOP still there stops again, without asking the AI.
    let mut cmd = p.decree(&["daemon", "--once", "--retry", &id, "--state", "implement"]);
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
        let mut cmd = p.decree(&["daemon", "--once", "--retry", &id, "--state", state]);
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
        let log = p.log(&id, "implement");
        assert!(
            log.contains(&format!("=== claude session {first} ===\n")),
            "{log}"
        );
        assert!(
            log.contains(&format!(
                "[Claude token limit] Usage limit reached. Waiting until {until} to retry.\n"
            )),
            "{log}"
        );
        assert!(
            log.contains(&format!("[Claude token limit] Resuming session {first}\n")),
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

/// A reset further away than AI_LIMIT_WAIT is not waited for: the call writes STOP saying
/// when to retry, and the run stops without sleeping or calling Claude again.
#[test]
fn a_reset_past_ai_limit_wait_stops_the_run() {
    let p = Project::init();
    p.stub("date", STUB_DATE);
    p.stub("sleep", STUB_SLEEP);
    let id = p.run(&[
        (
            "CLAUDE_LIMIT",
            "Claude AI usage limit reached. Limits reset at 10:00 PM",
        ),
        ("STUB_NOW", "23:00:00"),
    ]);
    assert_eq!(p.outcome(&id), "failed");
    assert_eq!(
        path(&p, &id)[1..],
        ["precheck -done-> implement", "implement -stop-> failed"]
    );
    assert!(!p.bin().join("slept").exists());
    assert_eq!(p.calls().len(), 1, "{:?}", p.calls());
    let stop = fs::read_to_string(p.run_dir(&id).join("STOP")).unwrap();
    assert!(
        stop.starts_with("Claude's usage limit is reached and resets at 22:00 (in 1380m 0s).\n"),
        "{stop}"
    );
    assert!(stop.contains("more than AI_LIMIT_WAIT (6h) away"), "{stop}");
    assert!(
        stop.contains(&format!("`decree daemon --once --retry {id}`")),
        "{stop}"
    );

    // A longer AI_LIMIT_WAIT waits it out.
    let p = Project::init();
    p.stub("date", STUB_DATE);
    p.stub("sleep", STUB_SLEEP);
    let id = p.run(&[
        (
            "CLAUDE_LIMIT",
            "Claude AI usage limit reached. Limits reset at 10:00 PM",
        ),
        ("STUB_NOW", "23:00:00"),
        ("AI_LIMIT_WAIT", "1d"),
    ]);
    assert_eq!(p.outcome(&id), "done");
    assert_eq!(
        fs::read_to_string(p.bin().join("slept")).unwrap(),
        "82800\n"
    );
}

/// `ai` calls the backend a data entry `ai_cli` names, else AI_CLI, else the first of claude,
/// opencode and copilot on PATH; an unknown one fails the call. Every call is a line in
/// .decree/ai-use.md.
#[test]
fn ai_picks_the_backend_and_logs_every_call() {
    let p = Project::init();
    p.stub(
        "opencode",
        "#!/usr/bin/env bash\necho \"$*\" | head -c 40 >> \"$(dirname \"$0\")/opencode_calls\"\necho >> \"$(dirname \"$0\")/opencode_calls\"\necho 'VERDICT: pass'\n",
    );
    let id = p.run(&[("AI_CLI", "opencode"), ("OPENCODE_MODEL", "llama.cpp/q")]);
    assert_eq!(p.outcome(&id), "done");
    assert!(p.calls().is_empty(), "{:?}", p.calls());
    let calls = fs::read_to_string(p.bin().join("opencode_calls")).unwrap();
    assert_eq!(calls.lines().count(), 2, "implement and verify: {calls}");
    assert!(
        calls.starts_with("run -m llama.cpp/q You are a senior engi"),
        "{calls}"
    );

    let log = fs::read_to_string(p.root().join(".decree/ai-use.md")).unwrap();
    let lines: Vec<&str> = log.lines().collect();
    assert_eq!(
        lines[0],
        "| time | run | state | cli | model | run attempt | attempt | seconds | exit |"
    );
    assert_eq!(lines.len(), 4, "{log}");
    let row: Vec<&str> = lines[2].split(" | ").collect();
    assert_eq!(row[1], id, "{log}");
    assert_eq!(
        row[2..8],
        [
            "develop/implement",
            "opencode",
            "llama.cpp/q",
            "1",
            "1",
            "0"
        ],
        "{log}"
    );
    assert_eq!(row[8], "0 |", "{log}");

    let id = p.run(&[("AI_CLI", "nobody")]);
    assert_eq!(p.outcome(&id), "failed");
    assert!(
        p.log(&id, "implement")
            .contains("ai: no ai_nobody function in ai.sh (AI_CLI=nobody)"),
        "{}",
        p.log(&id, "implement")
    );
}

/// A `PATH` of one folder under the project: links to the tools `ai.sh` uses, and a stub for
/// each of `clis`, which prints its name. No other AI CLI is on it, whatever this system has.
fn path_with_only(p: &Project, clis: &[&str]) -> PathBuf {
    let dir = p.root().join("only-bin");
    fs::create_dir_all(&dir).unwrap();
    for tool in [
        "bash", "date", "wc", "head", "tail", "mv", "tee", "grep", "cat", "dirname",
    ] {
        let found = std::process::Command::new("bash")
            .args(["-c", &format!("command -v {tool}")])
            .output()
            .unwrap();
        let target = String::from_utf8(found.stdout).unwrap();
        std::os::unix::fs::symlink(target.trim(), dir.join(tool)).unwrap();
    }
    for cli in clis {
        write_script(
            &dir.join(cli),
            &format!("#!/usr/bin/env bash\necho \"{cli} was asked\"\n"),
        );
    }
    dir
}

/// AC (A10): with no `AI_CLI` and no `ai_cli` data, `ai` uses the first of claude, opencode
/// and copilot on `PATH`: only opencode there, it uses opencode; none, it fails, naming the
/// three and `AI_CLI`. The router picks its CLI the same way.
#[test]
fn ai_without_ai_cli_uses_the_first_cli_on_path() {
    let p = Project::init();
    let run_dir = p.root().join("run");
    fs::create_dir_all(&run_dir).unwrap();
    let ask = |clis: &[&str]| {
        let path = path_with_only(&p, clis);
        let out = std::process::Command::new(path.join("bash"))
            .args(["-c", ". .decree/machines/ai/scripts/ai.sh && ai 'Say hi.'"])
            .current_dir(p.root())
            .env_clear()
            .env("PATH", &path)
            .env("HOME", p.root())
            .env("DECREE_RUN_DIR", &run_dir)
            .output()
            .unwrap();
        fs::remove_dir_all(&path).unwrap();
        out
    };
    let out = ask(&["opencode"]);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(String::from_utf8_lossy(&out.stdout), "opencode was asked\n");
    let log = fs::read_to_string(p.root().join(".decree/ai-use.md")).unwrap();
    assert!(
        log.lines().last().unwrap().contains(" | opencode | "),
        "{log}"
    );

    let out = ask(&["copilot", "opencode"]);
    assert_eq!(String::from_utf8_lossy(&out.stdout), "opencode was asked\n");

    let out = ask(&[]);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("no AI CLI: set AI_CLI") && stderr.contains("claude, opencode or copilot"),
        "{stderr}"
    );

    // The router: only opencode on PATH, it asks opencode.
    let path = path_with_only(&p, &[]);
    for tool in ["jq", "awk"] {
        let found = std::process::Command::new("bash")
            .args(["-c", &format!("command -v {tool}")])
            .output()
            .unwrap();
        let target = String::from_utf8(found.stdout).unwrap();
        std::os::unix::fs::symlink(target.trim(), path.join(tool)).unwrap();
    }
    write_script(
        &path.join("opencode"),
        "#!/usr/bin/env bash\necho '{\"event\": \"go\", \"reason\": \"r\", \"confidence\": 1}'\n",
    );
    let request = run_dir.join("request.json");
    fs::write(
        &request,
        serde_json::json!({
            "machine": "m", "machine_description": "", "state": "s", "state_description": "",
            "question": "Go?", "input": "", "message_body": "", "history": [],
            "options": [{"event": "go", "description": "Go."}],
        })
        .to_string(),
    )
    .unwrap();
    let out = std::process::Command::new(path.join("bash"))
        .arg(".decree/machines/router/scripts/ask.sh")
        .current_dir(p.root())
        .env_clear()
        .env("PATH", &path)
        .env("DECREE_REQUEST", &request)
        .env("DECREE_REPLY", run_dir.join("reply.json"))
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("--- reply (opencode)"),
        "{out:?}"
    );
    assert!(fs::read_to_string(run_dir.join("reply.json"))
        .unwrap()
        .contains("\"go\""));
}

/// A run's data entry `ai_cli` wins over the project's `AI_CLI`: with `AI_CLI=opencode` set
/// (as in `.decree/.env`), a child invoked with `params: { ai_cli: claude }` asks Claude, and a
/// machine without the entry follows `AI_CLI`.
#[test]
fn a_runs_ai_cli_wins_over_the_projects() {
    let p = Project::init();
    let decree = p.root().join(".decree");
    common::write_file(
        &decree.join("machines/ask/machine.yml"),
        "name: ask\ndescription: Ask once with the CLI ai_cli names.\n\
         data:\n  ai_cli: { type: string, default: opencode }\n\
         initial: say\nstates:\n  say: { invoke: say, transitions: { done: done } }\n  \
         done: { final: true }\n  failed: { final: true }\n",
    );
    common::write_file(
        &decree.join("machines/both/machine.yml"),
        "name: both\ndescription: Ask with the project's CLI, then a child with Claude.\n\
         initial: own\nstates:\n  own: { invoke: say, transitions: { done: child } }\n  \
         child:\n    invoke:\n      machine: { name: ask, params: { ai_cli: claude } }\n    \
         transitions: { done: done }\n  done: { final: true }\n  failed: { final: true }\n",
    );
    write_script(
        &decree.join("scripts/say.sh"),
        "#!/usr/bin/env bash\nset -euo pipefail\n. .decree/machines/ai/scripts/ai.sh\nai \"Say hi from $DECREE_MACHINE.\"\n",
    );
    p.stub(
        "opencode",
        "#!/usr/bin/env bash\necho \"${!#}\" >> \"$(dirname \"$0\")/opencode_calls\"\n",
    );
    let id = p.run_on("both", &[("AI_CLI", "opencode")]);
    assert_eq!(p.outcome(&id), "done", "{:?}", path(&p, &id));
    assert_eq!(
        fs::read_to_string(p.bin().join("opencode_calls")).unwrap(),
        "Say hi from both.\n"
    );
    let calls = p.calls();
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert!(calls[0].ends_with("| Say hi from ask."), "{calls:?}");
}

/// OpenCode's default `external_directory: ask` ends the session when it rejects a write
/// outside the project; `ai` continues the session, up to AI_NUDGES times, saying where
/// scratch files go.
#[test]
fn opencode_is_nudged_back_after_a_rejected_write() {
    let p = Project::init();
    // Rejects on the first two calls, then answers.
    p.stub(
        "opencode",
        r#"#!/usr/bin/env bash
dir=$(dirname "$0")
n=$(cat "$dir/n" 2>/dev/null || echo 0); echo $((n + 1)) > "$dir/n"
printf '%s
' "${*:1:$#-1} | $(head -n 1 <<<"${!#}")" >> "$dir/opencode_calls"
if [ "$n" -lt 2 ]; then
  echo '! permission requested: external_directory (/tmp/*); auto-rejecting'
  exit 0
fi
echo 'VERDICT: pass'
"#,
    );
    let id = p.run(&[("AI_CLI", "opencode")]);
    assert_eq!(p.outcome(&id), "done");
    let calls = fs::read_to_string(p.bin().join("opencode_calls")).unwrap();
    let calls: Vec<&str> = calls.lines().collect();
    assert!(
        calls[0].starts_with("run | You are a senior engineer"),
        "{calls:?}"
    );
    for call in &calls[1..3] {
        assert!(
            call.starts_with("run -c | A tool call you made was rejected because it used a path outside the project"),
            "{calls:?}"
        );
    }
    let log = p.log(&id, "implement");
    assert!(
        log.contains("=== opencode: a tool call was rejected; continuing the session (2/2) ==="),
        "{log}"
    );
    assert!(p.run_dir(&id).join("opencode.out").is_file());
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

/// infra: the agent declares a line, and apply checks every line, installing nothing that is
/// already there.
#[test]
fn infra_declares_a_resource_then_applies_it() {
    let p = Project::init();
    let id = p.run_on("infra", &[("CLAUDE_DECLARE", "command   ls")]);
    assert_eq!(p.outcome(&id), "done");
    assert_eq!(
        path(&p, &id),
        [
            "null -claimed-> declare",
            "declare -done-> apply",
            "apply -done-> done",
        ]
    );
    assert_eq!(
        p.log(&id, "apply"),
        "ok       command git (machines/infra/scripts/requirements)\n\
         ok       command ls (machines/infra/scripts/requirements)\n"
    );
    let calls = p.calls();
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert!(
        calls[0].contains("| You maintain this project's environment. Read "),
        "{calls:?}"
    );
}

/// A resource only a person can install fails infra's apply, naming it, and the declaration
/// is stashed with the run's changes; once the person has taken it back and committed it,
/// develop's precheck fails too until it is installed, before the agent runs.
#[test]
fn a_missing_resource_fails_infra_and_develop_precheck() {
    let p = Project::init();
    let id = p.run_on("infra", &[("CLAUDE_DECLARE", "command   not_a_tool_xyz")]);
    assert_eq!(p.outcome(&id), "failed");
    assert_eq!(
        p.log(&id, "apply"),
        "ok       command git (machines/infra/scripts/requirements)\n\
         not_a_tool_xyz is not on PATH: install it, then continue the run\n\
         FAILED   command not_a_tool_xyz (machines/infra/scripts/requirements)\n"
    );
    // The declaration stays in the tree with the rest of the failed run, and its attention
    // file holds the queue; the person commits it and deletes the file.
    assert!(p
        .log(&id, "failed")
        .contains(&format!("`decree daemon --once --retry {id}`")));
    fs::remove_file(p.attention(&id)).unwrap();
    let id = p.run(&[]);
    assert_eq!(p.outcome(&id), "failed");
    assert_eq!(
        path(&p, &id),
        ["null -claimed-> precheck", "precheck -error-> failed"]
    );
    assert_eq!(
        p.log(&id, "precheck"),
        "ok       command git (machines/infra/scripts/requirements)\n\
         missing  command not_a_tool_xyz (machines/infra/scripts/requirements)\n"
    );
    assert_eq!(p.calls().len(), 1, "only infra's declare");
}

/// AC (A08): `machines/develop/scripts/requirements` naming a command not on `PATH`, beside infra's
/// `git`, fails develop's precheck before the agent runs, naming the command and the file.
#[test]
fn precheck_checks_every_machines_requirements() {
    let p = Project::init();
    fs::write(
        p.root()
            .join(".decree/machines/develop/scripts/requirements"),
        "command   not_a_tool_xyz\n",
    )
    .unwrap();
    p.decree(&["plugin", "export", "develop"])
        .assert()
        .success();
    p.commit_all("develop's requirements");
    let id = p.run(&[]);
    assert_eq!(p.outcome(&id), "failed");
    assert_eq!(
        path(&p, &id),
        ["null -claimed-> precheck", "precheck -error-> failed"]
    );
    assert_eq!(
        p.log(&id, "precheck"),
        "ok       command git (machines/infra/scripts/requirements)\n\
         missing  command not_a_tool_xyz (machines/develop/scripts/requirements)\n"
    );
    assert!(p.calls().is_empty(), "{:?}", p.calls());
}

/// `CLAUDE_MODEL` picks the model for every call; without it, no `--model` is passed.
#[test]
fn claude_model_picks_the_model() {
    let p = Project::init();
    p.run(&[("CLAUDE_MODEL", "haiku")]);
    p.run(&[]);
    let calls = p.calls();
    assert_eq!(calls.len(), 4, "{calls:?}");
    for call in &calls[..2] {
        assert!(
            call.starts_with("-p --permission-mode auto --model haiku --session-id "),
            "{call}"
        );
    }
    for call in &calls[2..] {
        assert!(!call.contains("--model"), "{call}");
    }
}

/// A chaos suite in shell for the qa tests: every `tests/chaos/*.sh` must pass, and every
/// `*.xfail` must fail (a strict expected failure).
const CHAOS_RUN: &str = "for t in tests/chaos/*.sh; do bash \"$t\" || exit 1; done; \
                         for t in tests/chaos/*.xfail; do [ -e \"$t\" ] || continue; \
                         ! bash \"$t\" || exit 1; done";

/// The stub agent as qa's steps: recon writes surface.md; attack writes one passing and one
/// failing test; triage marks the failing one expected to fail and writes one bug report.
const QA_AGENT: &str = r#"
case "$prompt" in
  *"Write ${DECREE_RUN_DIR}/surface.md"*) echo "- input: I1" > "$DECREE_RUN_DIR/surface.md" ;;
  *"make this project misbehave"*)
    mkdir -p tests/chaos
    echo 'true' > tests/chaos/ok.sh
    echo 'echo "empty input crashes" >&2; false' > tests/chaos/empty.sh ;;
  *"You are QA triage"*)
    mv tests/chaos/empty.sh tests/chaos/empty.xfail
    printf '# Empty input crashes\n\nTest: tests/chaos/empty.xfail\n' > "$DECREE_RUN_DIR/bugs/empty-input.md" ;;
esac
"#;

impl Project {
    /// `init`, then `QA_RUN` set to `CHAOS_RUN`, and an app file.
    fn init_qa() -> Project {
        let p = Project::init();
        let qa = p.root().join(".decree/machines/qa/scripts/qa.sh");
        let text = fs::read_to_string(&qa).unwrap();
        let text = text.replacen("QA_RUN=\"\"\n", &format!("QA_RUN='{CHAOS_RUN}'\n"), 1);
        fs::write(&qa, text).unwrap();
        fs::write(p.root().join("app.txt"), "the app\n").unwrap();
        p.commit_all("qa setup");
        p
    }

    /// The ids of the runs on `machine`.
    fn runs_on(&self, machine: &str) -> Vec<String> {
        let mut ids: Vec<String> = fs::read_dir(self.root().join(".decree/runs"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|id| {
                fs::read_to_string(self.run_dir(id).join("message.md"))
                    .unwrap()
                    .contains(&format!("\nmachine: {machine}\n"))
            })
            .collect();
        ids.sort();
        ids
    }
}

/// qa end to end: its tests find a bug, triage marks it expected to fail, the chaos suite
/// passes, and report hands the bug to develop, which the same `process` runs.
#[test]
fn qa_hands_each_bug_to_develop_as_a_failing_test() {
    let p = Project::init_qa();
    let id = p.run_on("qa", &[("CLAUDE_DO", QA_AGENT)]);
    assert_eq!(p.outcome(&id), "done");
    assert_eq!(
        path(&p, &id),
        [
            "null -claimed-> recon",
            "recon -done-> attack",
            "attack -done-> run",
            "run -done-> triage",
            "triage -done-> report",
            "report -done-> done",
        ]
    );
    let chaos = fs::read_to_string(p.run_dir(&id).join("chaos.log")).unwrap();
    assert_eq!(chaos, "empty input crashes\n");
    assert!(
        p.log(&id, "report").starts_with("queued "),
        "{}",
        p.log(&id, "report")
    );
    let develop = p.runs_on("develop");
    assert_eq!(develop.len(), 1, "{develop:?}");
    let message = fs::read_to_string(p.run_dir(&develop[0]).join("message.md")).unwrap();
    assert!(message.contains(&format!("parent: {id}\n")), "{message}");
    assert!(
        message.ends_with("# Empty input crashes\n\nTest: tests/chaos/empty.xfail\n"),
        "{message}"
    );
    assert_eq!(p.outcome(&develop[0]), "done");
}

/// qa does not start, and asks no agent, until `QA_RUN` is set.
#[test]
fn qa_needs_its_chaos_suite_command_first() {
    let p = Project::init();
    let id = p.run_on("qa", &[]);
    assert_eq!(p.outcome(&id), "failed");
    assert_eq!(
        path(&p, &id),
        ["null -claimed-> recon", "recon -error-> failed"]
    );
    assert_eq!(
        p.log(&id, "recon"),
        "qa: no chaos suite command; set QA_RUN in .decree/machines/qa/scripts/qa.sh, then keep it with `decree plugin export qa`\n"
    );
    assert!(p.calls().is_empty());
}

/// An agent that changes a file outside the chaos tests fails its qa step, naming the file.
#[test]
fn qa_fails_a_step_whose_agent_changed_code() {
    let p = Project::init_qa();
    let id = p.run_on("qa", &[("CLAUDE_DO", "echo fixed >> app.txt")]);
    assert_eq!(p.outcome(&id), "failed");
    assert_eq!(
        path(&p, &id),
        ["null -claimed-> recon", "recon -error-> failed"]
    );
    assert!(
        p.log(&id, "recon").ends_with(
            "qa: the AI changed files outside tests/chaos; QA writes tests only:\napp.txt\n"
        ),
        "{}",
        p.log(&id, "recon")
    );
}

/// A failure the docs do not settle stops triage with the question, before report.
#[test]
fn qa_triage_stops_on_a_question() {
    let p = Project::init_qa();
    let agent = format!(
        "{QA_AGENT}\ncase \"$prompt\" in *\"You are QA triage\"*) echo 'Is empty input allowed?' > \"$DECREE_RUN_DIR/STOP\" ;; esac"
    );
    let id = p.run_on("qa", &[("CLAUDE_DO", agent.as_str())]);
    assert_eq!(p.outcome(&id), "failed");
    assert_eq!(path(&p, &id).last().unwrap(), "triage -stop-> failed");
    assert!(p.runs_on("develop").is_empty());
}

/// A `command` line with a version checks it in `--version`.
#[test]
fn a_command_line_checks_its_version() {
    let p = Project::init();
    let id = p.run_on(
        "infra",
        &[("CLAUDE_DECLARE", "command   bash   0.0.0-none")],
    );
    assert_eq!(p.outcome(&id), "failed");
    assert!(
        p.log(&id, "apply").ends_with(
            "bash 0.0.0-none is not on PATH: install it, then continue the run\n\
             FAILED   command bash 0.0.0-none (machines/infra/scripts/requirements)\n"
        ),
        "{}",
        p.log(&id, "apply")
    );
}

/// A run starts from a clean work tree: uncommitted changes fail it before anything runs,
/// naming them.
#[test]
fn a_dirty_tree_fails_the_run_before_the_agent() {
    let p = Project::init();
    fs::write(p.root().join("stray.txt"), "left over\n").unwrap();
    let out = p
        .decree(&["emit", "--machine", "develop"])
        .write_stdin(MESSAGE)
        .output()
        .unwrap();
    let id = String::from_utf8(out.stdout).unwrap().trim().to_string();
    p.decree(&["daemon", "--once"])
        .env("PATH", p.path())
        .output()
        .unwrap();
    assert_eq!(p.outcome(&id), "failed");
    assert_eq!(
        path(&p, &id),
        ["null -claimed-> precheck", "precheck -error-> failed"]
    );
    assert_eq!(
        clean_tree_log(&p, &id),
        "clean_tree: uncommitted changes; commit or stash them, then run the message again:\n\
         ?? stray.txt\n\
         if a failed run left them, its file in .decree/attention/ names it; continue it with \
         `decree daemon --once --retry <id>`\n"
    );
    assert!(p.calls().is_empty());
}

/// A run interrupted after its agent changed files never reaches `keep`; the next run's
/// `clean_tree` names it, with the command that continues it and the one that stashes its
/// changes, and writes the same lines to its `leftovers`, which its attention file quotes.
/// Once the changes are stashed, that run passes `clean_tree`.
#[test]
fn clean_tree_names_the_interrupted_run_that_left_changes() {
    let p = Project::init();
    p.commit_all("test setup");
    let first = p.emit_only();
    // The agent writes a file, then signals decree (the parent of the script's process
    // group), as Ctrl-C would.
    let interrupt = "case \"$prompt\" in *senior*) echo hi > hello.txt; \
         leader=$(ps -o pgid= -p $$ | tr -d ' '); \
         kill -INT \"$(ps -o ppid= -p \"$leader\" | tr -d ' ')\"; sleep 5 ;; esac";
    p.decree(&["daemon", "--once"])
        .env("PATH", p.path())
        .env("CLAUDE_DO", interrupt)
        .output()
        .unwrap();
    assert_eq!(p.events(&first).last().unwrap()["type"], "interrupted");
    assert_eq!(p.git(&["status", "--porcelain"]), "?? hello.txt\n");

    let calls = p.calls().len();
    let second = p.emit_only();
    p.decree(&["daemon", "--once"])
        .env("PATH", p.path())
        .output()
        .unwrap();
    assert_eq!(p.outcome(&second), "failed");
    let named = format!(
        "run {first} (Add a greeting) started from a clean tree and is interrupted; it may \
         have left them:\n  \
         to continue it on them: `decree daemon --once --retry {first}`\n  \
         to set them aside: `git stash push --include-untracked -m \"decree {first}\" -- .`\n"
    );
    assert_eq!(
        clean_tree_log(&p, &second),
        format!(
            "clean_tree: uncommitted changes; commit or stash them, then run the message \
             again:\n\
             ?? hello.txt\n\
             {named}\
             if a failed run left them, its file in .decree/attention/ names it; continue it \
             with `decree daemon --once --retry <id>`\n"
        )
    );
    assert_eq!(
        fs::read_to_string(p.run_dir(&second).join("leftovers")).unwrap(),
        named
    );
    let attention = fs::read_to_string(p.attention(&second)).unwrap();
    assert!(attention.contains(named.trim_end()), "{attention}");
    assert_eq!(p.calls().len(), calls, "no agent ran");

    // The person runs the printed stash command; the second run passes `clean_tree` now.
    p.git(&[
        "stash",
        "push",
        "--include-untracked",
        "-m",
        &format!("decree {first}"),
        "--",
        ".",
    ]);
    p.decree(&["daemon", "--once", "--retry", &second])
        .env("PATH", p.path())
        .output()
        .unwrap();
    assert_eq!(p.outcome(&second), "done");
    assert!(p.run_dir(&second).join("clean_tree").exists());
    assert!(!p.run_dir(&second).join("leftovers").exists());
    assert!(p
        .git(&["stash", "list"])
        .contains(&format!("decree {first}")));
}

/// A failed run whose attention file a person deleted (so it holds nothing) is named too;
/// a run that failed at `clean_tree` itself never started on the tree, and is not.
#[test]
fn clean_tree_names_a_failed_run_and_not_one_that_never_started() {
    let p = Project::init();
    let (first, _) = p.fail_with_changes();
    assert_eq!(p.outcome(&first), "failed");
    fs::remove_file(p.attention(&first)).unwrap();
    let second = p.run_on_dirty(&[]);
    let third = p.run_on_dirty(&[]);
    for id in [&second, &third] {
        assert_eq!(p.outcome(id), "failed");
        let log = clean_tree_log(&p, id);
        assert!(
            log.contains(&format!(
                "run {first} (Add a greeting) started from a clean tree and is failed; it may \
                 have left them:\n"
            )),
            "{log}"
        );
        assert!(log.contains(&format!("--retry {first}`")), "{log}");
        assert!(!log.contains(&format!("run {second}")), "{log}");
    }
}

/// The output of run `id`'s `clean_tree`.
fn clean_tree_log(p: &Project, id: &str) -> String {
    let events = p.events(id);
    let clean = events
        .iter()
        .rev()
        .find(|e| e["type"] == "script" && e["script"] == "clean_tree")
        .unwrap();
    fs::read_to_string(p.run_dir(id).join(clean["log"].as_str().unwrap())).unwrap()
}

/// Each machine's `done` commits the run's work, titled with the message's heading; a run that
/// changed nothing commits nothing.
#[test]
fn done_commits_the_runs_work() {
    let p = Project::init();
    let id = p.run(&[(
        "CLAUDE_DO",
        "case \"$prompt\" in *senior*) echo hi > hello.txt ;; esac",
    )]);
    assert_eq!(p.outcome(&id), "done");
    assert_eq!(p.commits()[0], "Add a greeting");
    assert_eq!(p.git(&["status", "--porcelain"]), "");
    assert_eq!(
        p.git(&["show", "--format=%b", "--stat", "HEAD"])
            .lines()
            .next()
            .unwrap(),
        format!("decree run {id} (develop)")
    );
    let before = p.commits().len();
    let id = p.run(&[]);
    assert_eq!(p.outcome(&id), "done");
    assert_eq!(p.commits().len(), before, "nothing to commit");

    // AC (A10): an infra run that adds a requirement line commits it in infra's plugin file,
    // which `commit` exported, and nothing under .decree/scripts/, which git ignores.
    let id = p.run_on("infra", &[("CLAUDE_DECLARE", "command   ls")]);
    assert_eq!(p.outcome(&id), "done");
    assert_eq!(
        p.git(&["show", "--format=", "--name-only", "HEAD"]),
        ".decree/plugins/infra.md\n"
    );
    let plugin = p.git(&["show", "HEAD:.decree/plugins/infra.md"]);
    assert!(
        plugin.contains("\n    command   git\n    command   ls\n"),
        "{plugin}"
    );
    // Of an installed machine, git tracks only its graph.
    assert!(p
        .git(&["ls-files", ".decree/machines"])
        .lines()
        .all(|f| f.ends_with("/graph.md")));
    assert_eq!(p.git(&["status", "--porcelain"]), "");
}

/// AC (A10): a develop run that edits its gate and then fails exports nothing: the plugin
/// file is unchanged, and the edited gate is still installed, where a retry builds on it.
#[test]
fn a_failed_run_exports_nothing_and_keeps_its_edit_installed() {
    let p = Project::init();
    let plugin = p.root().join(".decree/plugins/develop.md");
    let before = fs::read(&plugin).unwrap();
    let id = p.run(&[
        (
            "CLAUDE_DO",
            "case \"$prompt\" in *senior*) echo '# edited by the run' >> .decree/machines/develop/scripts/gate.sh ;; esac",
        ),
        ("CLAUDE_REPLY", "VERDICT: fail"),
    ]);
    assert_eq!(p.outcome(&id), "failed");
    assert_eq!(fs::read(&plugin).unwrap(), before);
    let gate = fs::read_to_string(p.script("gate")).unwrap();
    assert!(gate.ends_with("# edited by the run\n"), "{gate}");
    // The next process keeps the edit: the plugin file did not change, so nothing is installed
    // over it, and `decree check` passes.
    p.decree(&["check"]).assert().success();
    let gate = fs::read_to_string(p.script("gate")).unwrap();
    assert!(gate.ends_with("# edited by the run\n"), "{gate}");
}

/// An order file's run commits its work and its line in `state/order.git.txt` together,
/// titled with the file, since its run id is a time id; the body names the run.
#[test]
fn an_order_files_commit_names_it_and_holds_its_record() {
    let p = Project::init();
    p.decree(&["plugin", "add", "order"]).assert().success();
    let order = p.root().join(".decree/machines/develop/order");
    fs::create_dir_all(&order).unwrap();
    fs::write(order.join("01-greet.md"), MESSAGE).unwrap();
    p.commit_all("test setup");
    let out = p
        .decree(&["daemon", "--once"])
        .env("PATH", p.path())
        .env(
            "CLAUDE_DO",
            "case \"$prompt\" in *senior*) echo hi > hello.txt ;; esac",
        )
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let id = common::run_of(&p.root().join(".decree"), "01-greet.md");
    assert_eq!(p.outcome(&id), "done");
    assert_eq!(p.commits()[0], "Add a greeting (develop/01-greet.md)");
    assert_eq!(
        p.git(&["show", "--format=%b", "--name-only", "HEAD"]),
        format!(
            "decree run {id} (develop)\n\n\n.decree/machines/develop/state/order.git.txt\nhello.txt\n"
        )
    );
    // order's own record finds the line there already: nothing more to commit.
    assert_eq!(p.git(&["status", "--porcelain"]), "");
}

/// A failed run keeps its changes in the work tree and writes its `hold`, so its file in
/// `.decree/attention/` holds the queue: the next run waits, and a retry removes both before
/// the run continues on the kept changes.
#[test]
fn a_failed_run_keeps_its_changes_and_holds_the_queue_until_retried() {
    let p = Project::init();
    let (id, out) = p.fail_with_changes();
    assert_eq!(p.outcome(&id), "failed");
    assert_eq!(p.git(&["status", "--porcelain"]), "?? hello.txt\n", "kept");
    assert_eq!(p.git(&["stash", "list"]), "");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("\n1 run(s) need attention: .decree/attention/\n"),
        "{err}"
    );
    let file = fs::read_to_string(p.attention(&id)).unwrap();
    let frontmatter = schema::frontmatter_to_json(&file).unwrap();
    let validator = schema::validator(schema::ATTENTION_SCHEMA);
    assert_eq!(
        schema::errors(&validator, &frontmatter),
        Vec::<String>::new()
    );
    assert_eq!(frontmatter["holds"], true, "{file}");
    assert_eq!(frontmatter["status"], "failed", "{file}");
    assert_eq!(frontmatter["message"], "inbox", "{file}");
    assert!(file.contains("\n# Add a greeting\n"), "{file}");
    assert!(file.contains("?? hello.txt"), "the kept changes: {file}");
    assert!(
        file.contains(&format!("    decree daemon --once --retry {id}\n")),
        "{file}"
    );
    assert!(p.run_dir(&id).join("hold").is_file());
    assert!(p
        .log(&id, "failed")
        .contains(&format!("`decree daemon --once --retry {id}`")));

    // The next run waits: process names the file, starts nothing and exits 1.
    let next = p.emit_only();
    let out = p
        .decree(&["daemon", "--once"])
        .env("PATH", p.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("nothing runs while a file in .decree/attention/ holds the queue"),
        "{err}"
    );
    assert!(
        err.contains(&format!(".decree/attention/{id}.md:\n# Add a greeting")),
        "{err}"
    );
    assert!(!p.run_dir(&next).exists(), "not claimed");

    // A retry removes the hold and the attention file before the run continues on the kept
    // changes, and the queue goes on.
    let out = p
        .decree(&["daemon", "--once", "--retry", &id, "--state", "verify"])
        .env("PATH", p.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let removed = stdout
        .find(&format!("removed .decree/runs/{id}/hold\n"))
        .unwrap_or_else(|| panic!("{stdout}"));
    let started = stdout
        .find(&format!("▶ {id}"))
        .unwrap_or_else(|| panic!("{stdout}"));
    assert!(removed < started, "{stdout}");
    assert!(!p.run_dir(&id).join("hold").exists());
    assert!(!p.attention(&id).exists());
    assert_eq!(p.outcome(&id), "done");
    assert_eq!(p.commits()[0], "Add a greeting");
    assert_eq!(p.git(&["show", "HEAD:hello.txt"]), "hi\n");
    assert_eq!(p.outcome(&next), "done");
    assert_eq!(p.git(&["status", "--porcelain"]), "");
    assert_eq!(
        fs::read_dir(p.root().join(".decree/attention"))
            .unwrap()
            .count(),
        0
    );
}

/// To drop a held run instead, a person stashes its changes and deletes its attention file:
/// decree deletes the run's `hold` and the queue goes on.
#[test]
fn deleting_the_attention_file_of_a_held_run_releases_the_queue() {
    let p = Project::init();
    let (id, _) = p.fail_with_changes();
    assert!(p.run_dir(&id).join("hold").is_file());
    p.git(&["stash", "push", "--include-untracked", "--", "."]);
    fs::remove_file(p.attention(&id)).unwrap();

    let next = p.emit_only();
    let out = p
        .decree(&["daemon", "--once"])
        .env("PATH", p.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    assert!(!p.run_dir(&id).join("hold").exists());
    assert_eq!(p.outcome(&next), "done");
    // The run is still failed; its file stays deleted until its status changes.
    assert_eq!(p.outcome(&id), "failed");
    assert!(!p.attention(&id).exists());
}

/// A run that fails because the person's own changes are uncommitted leaves them alone.
#[test]
fn a_run_that_never_started_clean_keeps_and_holds_nothing() {
    let p = Project::init();
    fs::write(p.root().join("mine.txt"), "my work\n").unwrap();
    let out = p
        .decree(&["emit", "--machine", "develop"])
        .write_stdin(MESSAGE)
        .output()
        .unwrap();
    let id = String::from_utf8(out.stdout).unwrap().trim().to_string();
    p.decree(&["daemon", "--once"])
        .env("PATH", p.path())
        .output()
        .unwrap();
    assert_eq!(p.outcome(&id), "failed");
    assert_eq!(p.git(&["status", "--porcelain"]), "?? mine.txt\n");
    assert!(!p.run_dir(&id).join("hold").exists());
    let file = fs::read_to_string(p.attention(&id)).unwrap();
    assert!(file.contains("\nholds: false\n"), "{file}");
}

/// A project in a subfolder of a larger repository touches only its own folder: changes in a
/// sibling folder neither stop a run nor go into its commit, even when they are staged.
#[test]
fn a_project_in_a_subfolder_leaves_the_rest_of_the_repository_alone() {
    let p = Project::init_in("app");
    let sibling = p.tmp.path().join("other");
    fs::create_dir(&sibling).unwrap();
    fs::write(sibling.join("staged.txt"), "theirs\n").unwrap();
    fs::write(sibling.join("loose.txt"), "theirs\n").unwrap();
    p.git(&["add", "../other/staged.txt"]);
    let id = p.run_on_dirty(&[(
        "CLAUDE_DO",
        "case \"$prompt\" in *senior*) echo hi > hello.txt ;; esac",
    )]);
    assert_eq!(p.outcome(&id), "done");
    assert_eq!(
        p.git(&["show", "--format=", "--name-only", "HEAD"]),
        "app/hello.txt\n"
    );
    assert_eq!(
        p.git(&["status", "--porcelain"]),
        "A  other/staged.txt\n?? other/loose.txt\n"
    );
}

/// docs/escalation.md, installed as it says in a project `init` wrote: it passes
/// `decree check`, and a run whose verify fails spends `max_gap_rounds` rounds with the
/// default agent (OpenCode, through `gaps`' data entry `ai_cli`), then one with Claude, which
/// closes the gaps.
#[test]
fn the_documented_escalation_ladder_runs_rounds_then_claude() {
    let p = Project::init();
    let docs = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/escalation.md"))
        .unwrap();
    let file = regex::Regex::new(r"(?s)`(\.decree/[^`]+)`[^\n]*\n\n```\w*\n(.*?)```\n").unwrap();
    let mut written = Vec::new();
    for c in file.captures_iter(&docs) {
        let path = p.root().join(&c[1]);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        if c[1].ends_with(".sh") {
            write_script(&path, &c[2]);
        } else {
            fs::write(&path, &c[2]).unwrap();
        }
        written.push(c[1].to_string());
    }
    assert_eq!(written.len(), 5, "{written:?}");
    let start = docs
        .find("```bash\nmkdir -p .decree/machines/gaps/scripts")
        .unwrap()
        + "```bash\n".len();
    let links = &docs[start..start + docs[start..].find("```").unwrap()];
    let out = std::process::Command::new("bash")
        .args(["-c", links])
        .current_dir(p.root())
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    p.decree(&["graph"]).assert().success();
    let out = p.decree(&["check"]).output().unwrap();
    assert!(out.status.success(), "{out:?}");

    // OpenCode always finds gaps; Claude's develop verify fails once, its round passes.
    p.stub(
        "opencode",
        "#!/usr/bin/env bash\necho \"${!#}\" | head -n 1 >> \"$(dirname \"$0\")/opencode_calls\"\necho 'VERDICT: fail'\n",
    );
    let verdicts = "case \"$prompt\" in *VERDICT*) \
        n=$(cat \"$dir/verifies\" 2>/dev/null || echo 0); echo $((n + 1)) > \"$dir/verifies\"; \
        if [ \"$n\" = 0 ]; then echo '- the greeting is missing' > \"$DECREE_ROOT_RUN_DIR/gaps.md\"; \
        echo 'VERDICT: fail'; else echo 'VERDICT: pass'; fi ;; esac";
    let id = p.run(&[("CLAUDE_DO", verdicts), ("CLAUDE_REPLY", "")]);
    assert_eq!(p.outcome(&id), "done", "{:?}", path(&p, &id));
    assert_eq!(
        path(&p, &id)[4..],
        [
            "verify -fail-> rounds_left",
            "rounds_left -true-> fill_gaps",
            "fill_gaps -unfilled-> rounds_left",
            "rounds_left -true-> fill_gaps",
            "fill_gaps -unfilled-> rounds_left",
            "rounds_left -false-> claude_round",
            "claude_round -filled-> done",
        ]
    );
    let opencode = fs::read_to_string(p.bin().join("opencode_calls")).unwrap();
    assert_eq!(
        opencode
            .lines()
            .filter(|l| l.starts_with("You are a senior engineer."))
            .count(),
        2,
        "one fill per cheap round: {opencode}"
    );
    let progress = fs::read_to_string(p.run_dir(&id).join("progress.md")).unwrap();
    for heading in [
        "## Gaps, round 1 (ai_cli: opencode)",
        "## Gaps, round 2 (ai_cli: opencode)",
        "## Gaps, round 3 (ai_cli: claude)",
    ] {
        assert!(progress.contains(heading), "{progress}");
    }
    assert!(
        !p.run_dir(&id).join("gaps.md").exists(),
        "a pass removes it"
    );
    let log = fs::read_to_string(p.root().join(".decree/ai-use.md")).unwrap();
    assert!(log.contains("| gaps/fill | opencode |"), "{log}");
    assert!(log.contains("| gaps/fill | claude |"), "{log}");
}

/// A project `init` wrote, with the consensus router of docs/routers.md, one stub router per
/// router it asks (each picks the next of `picks`, after checking the consensus router's
/// request offers it), and a machine `pick` whose `decide` state asks it, sending `unsure`
/// to a person. `decree check` passes.
fn consensus_project(picks: [&str; 3]) -> Project {
    let p = Project::init();
    let docs =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/routers.md")).unwrap();
    let section = &docs[docs.find("## Consensus of several routers").unwrap()..];
    let file = regex::Regex::new(r"(?s)`(\.decree/[^`]+)`[^\n]*\n\n```\w*\n(.*?)```\n").unwrap();
    let mut written = Vec::new();
    for c in file.captures_iter(section) {
        let path = p.root().join(&c[1]);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        if c[1].ends_with(".sh") {
            write_script(&path, &c[2]);
        } else {
            fs::write(&path, &c[2]).unwrap();
        }
        written.push(c[1].to_string());
    }
    assert_eq!(
        written,
        [
            ".decree/machines/consensus_router/machine.yml",
            ".decree/machines/consensus_router/scripts/combine.sh"
        ]
    );
    let machine = fs::read_to_string(p.root().join(&written[0])).unwrap();
    let routers: Vec<String> = regex::Regex::new(r"machine: (\w+)")
        .unwrap()
        .captures_iter(&machine)
        .map(|c| c[1].to_string())
        .collect();
    assert_eq!(routers.len(), picks.len(), "{routers:?}");
    for (router, pick) in routers.iter().zip(picks) {
        common::write_file(
            &p.root()
                .join(format!(".decree/machines/{router}/machine.yml")),
            format!(
                "name: {router}\ndescription: Pick {pick}.\ninitial: ask\nstates:\n  \
                 ask:\n    invoke: ask\n    transitions: {{ done: done }}\n  \
                 done:   {{ final: true }}\n  failed: {{ final: true }}\n"
            ),
        );
        let dir = p.root().join(format!(".decree/machines/{router}/scripts"));
        fs::create_dir_all(&dir).unwrap();
        write_script(
            &dir.join("ask.sh"),
            &format!(
                "#!/usr/bin/env bash\nset -euo pipefail\n\
                 request=${{DECREE_REQUEST:-$DECREE_PARENT_RUN_DIR/request.json}}\n\
                 reply=${{DECREE_REPLY:-$DECREE_RUN_DIR/reply.json}}\n\
                 jq -e --arg pick {pick} '.options | map(.event) | index($pick)' \"$request\" > /dev/null\n\
                 jq -n --arg pick {pick} '{{event: $pick}}' > \"$reply\"\n"
            ),
        );
    }
    let option = |e: &str| format!("      {e}: {{ target: done, description: The letter {e}. }}\n");
    let options: String = ["a", "b", "c"].iter().map(|e| option(e)).collect();
    common::write_file(&p.root().join(".decree/machines/pick/machine.yml"), format!(
            "name: pick\ndescription: Pick a letter.\ninitial: decide\nstates:\n  \
             decide:\n    invoke:\n      model:\n        question: Which letter?\n        \
             router: consensus_router\n        min_confidence: 0.6\n    transitions:\n{options}      \
             unsure: ask_person\n  \
             ask_person:\n    invoke:\n      reply:\n        question: Which letter?\n        \
             notify: tell\n    transitions:\n{options}  \
             done:   {{ final: true }}\n  failed: {{ final: true }}\n"
        ));
    fs::create_dir_all(p.root().join(".decree/machines/pick/scripts")).unwrap();
    write_script(
        &p.root().join(".decree/machines/pick/scripts/tell.sh"),
        "#!/usr/bin/env bash\necho \"asked $DECREE_WAIT_ID\"\n",
    );
    p.decree(&["graph"]).assert().success();
    let out = p.decree(&["check"]).output().unwrap();
    assert!(out.status.success(), "{out:?}");
    p
}

/// The consensus router of docs/routers.md, with `quorum` 2: two of three routers picking
/// `a` is the pick, at a confidence of 2/3.
#[test]
fn the_documented_consensus_router_takes_the_quorums_pick() {
    let p = consensus_project(["a", "a", "b"]);
    let id = p.run_on("pick", &[]);
    assert_eq!(p.outcome(&id), "done", "{:?}", path(&p, &id));
    assert_eq!(path(&p, &id)[1..], ["decide -a-> done"]);
    let decision = p
        .events(&id)
        .into_iter()
        .find(|e| e["type"] == "decision")
        .unwrap();
    assert_eq!(decision["router"], "consensus_router");
    assert_eq!(decision["pick"], "a");
    let confidence = decision["confidence"].as_f64().unwrap();
    assert!((confidence - 2.0 / 3.0).abs() < 1e-9, "{decision}");
}

/// Three routers picking `a`, `b` and `c` is a split: the state's event is `unsure`, and a
/// person is asked.
#[test]
fn the_documented_consensus_router_sends_a_split_to_a_person() {
    let p = consensus_project(["a", "b", "c"]);
    let id = p.run_on("pick", &[]);
    assert_eq!(path(&p, &id)[1..], ["decide -unsure-> ask_person"]);
    let decision = p
        .events(&id)
        .into_iter()
        .find(|e| e["type"] == "decision")
        .unwrap();
    assert_eq!(decision["confidence"], 0.0, "{decision}");
    assert!(
        decision["reason"]
            .as_str()
            .unwrap()
            .starts_with("no 2 of 3 routers agree"),
        "{decision}"
    );
    let events = p.events(&id);
    let last = events.last().unwrap();
    assert_eq!(last["type"], "waiting", "{events:?}");
    assert_eq!(last["state"], "ask_person");
}

/// The `release` kind in docs/reference/scripts.md, run by `machines/infra/scripts/run.sh apply` from a
/// local archive: installed when the sha256 in its source matches, refused when it does not
/// or when the source has none.
#[test]
fn the_documented_release_kind_checks_its_sha256() {
    let p = Project::init();
    let docs =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/reference/scripts.md"))
            .unwrap();
    let start = docs.find("```bash\n# kinds/release.sh").unwrap() + "```bash\n".len();
    let kind = &docs[start..start + docs[start..].find("```").unwrap()];
    let infra = p.root().join(".decree/machines/infra/scripts");
    fs::write(infra.join("kinds/release.sh"), kind).unwrap();

    // An archive holding `mytool`, which prints its version.
    let pkg = p.root().join("pkg");
    fs::create_dir(&pkg).unwrap();
    write_script(&pkg.join("mytool"), "#!/bin/sh\necho mytool 1.2.3\n");
    let archive = p.root().join("mytool-1.2.3.tar.gz");
    let sh = |cmd: &str| {
        let out = std::process::Command::new("sh")
            .args(["-c", cmd])
            .current_dir(p.root())
            .output()
            .unwrap();
        assert!(out.status.success(), "{cmd}: {out:?}");
        String::from_utf8(out.stdout).unwrap()
    };
    sh("tar -czf mytool-1.2.3.tar.gz -C pkg mytool");
    let sum = sh("sha256sum mytool-1.2.3.tar.gz");
    let sum = sum.split_whitespace().next().unwrap();
    let url = format!("file://{}", archive.display()).replace("1.2.3", "{version}");
    let requirements = fs::read_to_string(infra.join("requirements")).unwrap();
    let apply = |source: &str| {
        fs::write(
            infra.join("requirements"),
            format!("{requirements}release   mytool   1.2.3   {source}\n"),
        )
        .unwrap();
        let _ = fs::remove_dir_all(p.root().join(".tools"));
        let out = std::process::Command::new(infra.join("run.sh"))
            .arg("apply")
            .current_dir(p.root())
            .env("PATH", p.path())
            .output()
            .unwrap();
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
        )
    };

    let (ok, out) = apply(&format!("{url}#sha256={sum}"));
    assert!(ok, "{out}");
    assert!(
        out.ends_with("changed  release mytool 1.2.3 (machines/infra/scripts/requirements)\n"),
        "{out}"
    );

    let (ok, out) = apply(&format!("{url}#sha256={}", "0".repeat(64)));
    assert!(!ok, "{out}");
    assert!(
        out.ends_with("FAILED   release mytool 1.2.3 (machines/infra/scripts/requirements)\n"),
        "{out}"
    );
    assert!(!p.root().join(".tools/bin/mytool").exists());

    let (ok, out) = apply(&url);
    assert!(!ok, "{out}");
    assert!(out.contains("mytool: no #sha256= in its source\n"), "{out}");
}
