//! The 0.5 CLI end to end (docs/reference/cli.md): `init`, `emit`, `process`,
//! `status`, `tail`, `retry` and `event` driven through the binary. Each test runs
//! `decree init` in its own temp directory, then adds the machines and scripts it needs.
//! No test calls a model: routers are test machines whose script writes a fixed reply.

use assert_cmd::cargo::cargo_bin_cmd;
use assert_cmd::Command;
use serde_json::Value;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

mod common;
use common::write_script;

/// Fails until `ok.flag` exists in the project root.
const FLAKY: &str = "\
name: flaky
description: Work that fails until ok.flag exists.
initial: work
states:
  work:
    invoke: work
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
";

const FLAKY_WORK: &str =
    "#!/usr/bin/env bash\necho working\n[ -e \"$DECREE_PROJECT_ROOT/ok.flag\" ]\n";

/// Asks a person whether to ship.
const DEPLOY: &str = "\
name: deploy
description: Ask a person before shipping.
initial: approval
states:
  approval:
    description: Ship the build?
    invoke:
      person:
        question: Ship it?
        ask: ask_person
    transitions:
      approve: { target: shipped, description: Ship it. }
      reject: { target: done, description: Do not ship. }
  shipped: { final: true }
  done: { final: true }
  failed: { final: true }
";

/// Builds, then asks a router machine what to do next.
const TRIAGE: &str = "\
name: triage
description: Build, then let a router decide.
initial: build
states:
  build:
    invoke: build
    transitions: { done: decide }
  decide:
    description: Decide what to do with the build.
    invoke:
      model:
        question: Ship or rework?
        router: test_router
        output: build
    transitions:
      ship: { target: done, description: The build is good. }
      rework: { target: failed, description: The build needs work. }
  done: { final: true }
  failed: { final: true }
";

const TEST_ROUTER: &str = "\
name: test_router
description: A router whose one script writes a fixed reply.
initial: ask
states:
  ask:
    invoke: reply
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
";

/// Writes the router's reply: `ship`.
const REPLY_LINE: &str =
    "echo '{\"event\":\"ship\",\"reason\":\"Tests pass.\",\"confidence\":0.9}' > \"$DECREE_REPLY\"\n";

/// Prints a line each second for five seconds.
const TICK: &str = "\
name: tick
description: A script that prints a line each second.
initial: work
states:
  work:
    invoke: ticker
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
";

const TICK_SH: &str =
    "#!/usr/bin/env bash\nfor i in 1 2 3 4 5; do echo \"tick $i\"; sleep 1; done\n";

/// Ticks while `tick.flag` exists in the project root.
const TICK_WHILE_FLAG: &str = "#!/usr/bin/env bash\nwhile [ -e \"$DECREE_PROJECT_ROOT/tick.flag\" ]; do echo tick; sleep 0.02; done\n";

struct Project {
    tmp: TempDir,
}

impl Project {
    /// `decree init --ai claude` in a new temp directory.
    fn init() -> Project {
        let p = Project {
            tmp: TempDir::new().unwrap(),
        };
        p.decree(&["init", "--ai", "claude"]).assert().success();
        p
    }

    fn root(&self) -> PathBuf {
        self.tmp.path().to_path_buf()
    }

    fn decree_dir(&self) -> PathBuf {
        self.root().join(".decree")
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

    /// `decree <args>` in the background, its stdout piped.
    fn spawn(&self, args: &[&str]) -> Child {
        std::process::Command::new(env!("CARGO_BIN_EXE_decree"))
            .current_dir(self.root())
            .env("NO_COLOR", "1")
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap()
    }

    fn machine(&self, name: &str, yml: &str) {
        fs::write(self.decree_dir().join(format!("machines/{name}.yml")), yml).unwrap();
    }

    fn script(&self, name: &str, text: &str) {
        write_script(&self.decree_dir().join("scripts").join(name), text);
    }

    /// `decree emit --machine <machine>` with `body` on stdin; returns the new id.
    fn emit(&self, machine: &str, body: &str) -> String {
        let out = self
            .decree(&["emit", "--machine", machine])
            .write_stdin(body)
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        String::from_utf8(out).unwrap().trim().to_string()
    }

    fn stdout(&self, args: &[&str], code: i32) -> String {
        let out = self.decree(args).assert().code(code).get_output().clone();
        String::from_utf8(out.stdout).unwrap()
    }

    fn run_dir(&self, id: &str) -> PathBuf {
        self.decree_dir().join("runs").join(id)
    }

    fn events(&self, id: &str) -> Vec<Value> {
        fs::read_to_string(self.run_dir(id).join("events.jsonl"))
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    fn runs(&self) -> Vec<String> {
        let mut ids: Vec<String> = fs::read_dir(self.decree_dir().join("runs"))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        ids.sort();
        ids
    }

    /// Wait until run `id` has a `.running`, and return the only run's id.
    fn wait_running(&self) -> String {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(id) = self
                .runs()
                .into_iter()
                .find(|id| self.run_dir(id).join(".running").exists())
            {
                return id;
            }
            assert!(Instant::now() < deadline, "no script started");
            thread::sleep(Duration::from_millis(5));
        }
    }
}

/// The `to` of each `transition` event with its source: `"from -> to (source)"`.
fn transitions(events: &[Value]) -> Vec<String> {
    events
        .iter()
        .filter(|e| e["type"] == "transition")
        .map(|e| {
            format!(
                "{} -> {} ({})",
                e["from"].as_str().unwrap_or("-"),
                e["to"].as_str().unwrap(),
                e["source"].as_str().unwrap()
            )
        })
        .collect()
}

fn wait_exit(child: &mut Child, timeout: Duration) -> std::process::ExitStatus {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        assert!(Instant::now() < deadline, "decree did not exit in time");
        thread::sleep(Duration::from_millis(5));
    }
}

/// AC: `decree init`, `decree emit`, `decree process`, then `decree status <id>`: the run
/// shows `done`, with its script durations.
#[test]
fn init_emit_process_status_shows_the_run_done() {
    let p = Project::init();
    p.machine(
        "hello",
        &fs::read_to_string("examples/project/.decree/machines/hello.yml").unwrap(),
    );
    p.script("greet", "#!/usr/bin/env bash\necho hello\n");
    let id = p.emit("hello", "Say hello.\n");
    assert!(p.decree_dir().join(format!("inbox/{id}.md")).is_file());

    p.decree(&["process"]).assert().success();

    let status = p.stdout(&["status", &id], 0);
    assert!(status.contains("  machine: hello\n"), "{status}");
    assert!(
        status.contains("  status: finished in `done`\n"),
        "{status}"
    );
    assert!(status.contains("Frontmatter:\n  id: "), "{status}");
    assert!(status.contains("  state: done\n"), "{status}");
    assert!(status.contains("· --claimed--> greet (claim)"), "{status}");
    let script = status
        .lines()
        .find(|l| l.contains("greet/greet (invoke, attempt 1) exit 0 in "))
        .unwrap_or_else(|| panic!("no script line: {status}"));
    assert!(script.ends_with(", log 0001-greet-greet.log"), "{script}");
    assert!(status.contains("greet --done--> done (exit_code), exit 0"));
    assert!(status.contains("run_finished done after "), "{status}");

    let overview = p.stdout(&["status"], 0);
    assert!(
        overview.contains(&format!(
            "  finished: 1\n    done: 1\n      {id}  hello  `done`\n"
        )),
        "{overview}"
    );
    assert!(overview.contains("  inbox/: 0\n"), "{overview}");
}

/// AC: a failed run; `decree process --retry <id>` resumes it at the retried state, in
/// the same invocation. Also the docs/reference/cli.md exit codes of `--retry`.
#[test]
fn process_retry_resumes_a_failed_run_at_the_retried_state() {
    let p = Project::init();
    p.machine("flaky", FLAKY);
    p.script("work", FLAKY_WORK);
    let id = p.emit("flaky", "Do the work.\n");

    // The run fails: `process` stops on it with 1 and names the command that continues it.
    let out = p.decree(&["process"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains(&format!("`decree process --retry {id}`")),
        "{err}"
    );
    assert_eq!(
        transitions(&p.events(&id)),
        ["- -> work (claim)", "work -> failed (exit_code)"]
    );
    let status = p.stdout(&["status"], 0);
    assert!(
        status.contains(&format!(
            "    failed: 1\n      {id}  flaky  `failed`\n        \
             continue with `decree process --retry {id}`\n"
        )),
        "{status}"
    );
    let one = p.stdout(&["status", &id], 0);
    assert!(
        one.contains(&format!("  continue with `decree process --retry {id}`\n")),
        "{one}"
    );
    // A finished run cannot be retried into a final, unknown or compound state.
    for bad in ["done", "nowhere"] {
        p.decree(&["process", "--retry", &id, "--state", bad])
            .assert()
            .code(1);
    }
    p.decree(&["process", "--retry", "no-such-run"])
        .assert()
        .code(1);
    assert_eq!(p.events(&id).len(), 4, "a refused retry appends nothing");

    fs::write(p.root().join("ok.flag"), "").unwrap();
    let out = p.stdout(&["process", "--retry", &id], 0);
    assert!(
        out.contains(&format!("run {id} continues in `work`")),
        "{out}"
    );
    let events = p.events(&id);
    assert_eq!(
        transitions(&events),
        [
            "- -> work (claim)",
            "work -> failed (exit_code)",
            "failed -> work (retry)",
            "work -> done (exit_code)"
        ]
    );
    let retry = events.iter().find(|e| e["source"] == "retry").unwrap();
    assert_eq!(retry["event"], "retry");
    assert_eq!(retry["exit_code"], Value::Null);
    assert_eq!(events.last().unwrap()["type"], "run_finished");
    assert!(p
        .stdout(&["status", &id], 0)
        .contains("status: finished in `done`"));

    // A pending run (the `retry` transition written, not yet continued) is not retried
    // again: `process` continues it.
    let events_path = p.run_dir(&id).join("events.jsonl");
    let before = fs::read_to_string(&events_path).unwrap();
    let mut pending = retry.clone();
    pending["seq"] = (events.len() + 1).into();
    pending["from"] = "done".into();
    fs::write(&events_path, format!("{before}{pending}\n")).unwrap();
    let status = p.stdout(&["status"], 0);
    assert!(
        status.contains(&format!("  pending: 1\n    {id}  flaky  `work`\n")),
        "{status}"
    );
    p.decree(&["process", "--retry", &id]).assert().code(1);
    assert_eq!(
        fs::read_to_string(&events_path).unwrap().lines().count(),
        events.len() + 1
    );
}

/// AC: `--retry` with nothing to retry exits 1 and says so; `--retry` with `--dry-run`,
/// `--state` without `--retry`, and the removed `retry` command are usage errors (exit 2).
#[test]
fn process_retry_with_nothing_to_retry_and_its_usage_errors() {
    let p = Project::init();
    let out = p.decree(&["process", "--retry"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("nothing to retry: no migration is failed or interrupted"),
        "{err}"
    );
    for args in [
        &["process", "--retry", "--dry-run"][..],
        &["process", "--dry-run", "--retry", "some-run"],
        &["process", "--state", "work"],
        &["process", "--retry", "--format", "json"],
        &["retry"],
        &["retry", "some-run"],
    ] {
        p.decree(args).assert().code(2);
    }
}

/// AC: pending messages; `decree process --dry-run` lists them and nothing runs.
#[test]
fn dry_run_lists_pending_messages_and_runs_nothing() {
    let p = Project::init();
    p.machine("flaky", FLAKY);
    p.script("work", FLAKY_WORK);
    let id = p.emit("flaky", "Do the work.\n");
    fs::write(
        p.decree_dir().join("migrations/01-first.md"),
        "---\nmachine: flaky\n---\nFirst.\n",
    )
    .unwrap();

    let out = p.stdout(&["process", "--dry-run"], 0);
    assert!(out.contains("migrations/:\n  01-first.md"), "{out}");
    assert!(out.contains(&format!("inbox/:\n  {id}.md")), "{out}");
    assert!(out.contains("→ flaky"), "{out}");
    assert!(p.runs().is_empty());
    assert!(p.decree_dir().join(format!("inbox/{id}.md")).is_file());
    assert_eq!(
        fs::read_to_string(p.decree_dir().join("processed.md")).unwrap(),
        ""
    );
}

/// AC: a run that reaches a `person` state. `decree process` prints the wait id,
/// the options and a `decree event` command per option, and exits 0; after `decree event`
/// and another `decree process`, the run is `done`.
#[test]
fn person_prints_the_wait_and_a_reply_finishes_the_run() {
    let p = Project::init();
    p.machine("deploy", DEPLOY);
    p.script("ask_person", "#!/usr/bin/env bash\nexit 0\n");
    let id = p.emit("deploy", "Ship v1.\n");

    let out = p.stdout(&["process"], 0);
    let wait_id = p.events(&id).last().unwrap()["wait_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        out,
        format!(
            "Waiting: run {id} in `approval`: Ship the build?\n  \
             wait id {wait_id}, options: approve, reject\n  \
             decree event {wait_id} approve\n  decree event {wait_id} reject\n"
        )
    );
    let status = p.stdout(&["status"], 0);
    assert!(
        status.contains(&format!(
            "  waiting: 1\n    {id}  deploy  `approval`\n      wait id {wait_id}, options: approve, reject\n"
        )),
        "{status}"
    );
    // A waiting run takes a reply, not a retry.
    p.decree(&["process", "--retry", &id]).assert().code(1);

    p.decree(&["event", &wait_id, "approve", "-m", "Go."])
        .assert()
        .success();
    p.stdout(&["process"], 0);
    assert_eq!(
        transitions(&p.events(&id)),
        ["- -> approval (claim)", "approval -> shipped (person)"]
    );
    let status = p.stdout(&["status", &id], 0);
    assert!(status.contains("status: finished in `shipped`"), "{status}");
    assert!(
        status.contains("approval: person → approve (reply "),
        "{status}"
    );
}

/// AC: SIGINT during a run under `decree process`: exit 130, the run is `interrupted`, and
/// a later `decree process` leaves it alone until `decree process --retry` continues it.
#[test]
fn sigint_under_process_exits_130_and_leaves_the_run_interrupted() {
    let p = Project::init();
    p.machine("tick", TICK);
    p.script("ticker", TICK_WHILE_FLAG);
    let id = p.emit("tick", "Tick.\n");
    fs::write(p.root().join("tick.flag"), "").unwrap();

    let mut decree = p.spawn(&["process"]);
    assert_eq!(p.wait_running(), id);
    // SAFETY: sends SIGINT to the decree process only; the script has its own group.
    unsafe { libc::kill(decree.id() as i32, libc::SIGINT) };
    let status = wait_exit(&mut decree, Duration::from_secs(15));
    assert_eq!(status.code(), Some(130));

    let events = p.events(&id);
    let last = events.last().unwrap();
    assert_eq!(last["type"], "interrupted");
    assert_eq!(last["cause"], "signal");
    assert_eq!(last["script"], "ticker");
    assert!(!p.run_dir(&id).join(".running").exists());
    let overview = p.stdout(&["status"], 0);
    assert!(
        overview.contains(&format!(
            "  interrupted: 1\n    {id}  tick  `work`\n      continue with `decree process --retry {id}`\n"
        )),
        "{overview}"
    );

    p.decree(&["process"]).assert().success();
    assert_eq!(p.events(&id), events);

    // `decree process --retry` continues it and finishes it.
    fs::remove_file(p.root().join("tick.flag")).unwrap();
    p.decree(&["process", "--retry", &id]).assert().success();
    assert_eq!(p.events(&id).last().unwrap()["type"], "run_finished");
    assert!(p
        .stdout(&["status", &id], 0)
        .contains("status: finished in `done`"));
}

/// AC: a finished run; `decree status <id>` shows the transitions, each script with its
/// duration, and router decisions.
#[test]
fn status_of_a_finished_run_shows_transitions_scripts_and_router_decisions() {
    let p = Project::init();
    p.machine("triage", TRIAGE);
    p.machine("test_router", TEST_ROUTER);
    p.script("build", "#!/usr/bin/env bash\necho built\n");
    p.script("reply", &format!("#!/usr/bin/env bash\n{REPLY_LINE}"));
    let id = p.emit("triage", "Build it.\n");

    p.decree(&["process"]).assert().success();
    let child = p
        .runs()
        .into_iter()
        .find(|r| *r != id)
        .expect("the router's child run");

    let status = p.stdout(&["status", &id], 0);
    for want in [
        "status: finished in `done`",
        "· --claimed--> build (claim)",
        "build --done--> decide (exit_code), exit 0",
        "decide --ship--> done (model)",
        &format!(
            "decide: model → ship (router test_router, run {child}, pick ship, confidence 0.9, "
        ),
        "reason: Tests pass.",
        &format!("decide: waiting for child run {child}"),
        "run_finished done after ",
    ] {
        assert!(status.contains(want), "missing {want:?} in:\n{status}");
    }
    let script = status
        .lines()
        .find(|l| l.contains("build/build (invoke, attempt 1) exit 0 in "))
        .unwrap_or_else(|| panic!("no script line: {status}"));
    assert!(script.ends_with(", log 0001-build-build.log"), "{script}");

    // The router run is an ordinary run, with its own script and duration.
    let router = p.stdout(&["status", &child], 0);
    assert!(
        router.contains("ask/reply (invoke, attempt 1) exit 0 in "),
        "{router}"
    );
}

/// AC: a run whose script sleeps 5 s while printing a line each second. `decree status`
/// shows the script, its pid, elapsed time and log path; `decree tail` prints each line as
/// it is written and exits when the run finishes.
#[test]
fn status_and_tail_follow_a_script_while_it_runs() {
    let p = Project::init();
    p.machine("tick", TICK);
    p.script("ticker", TICK_SH);
    let id = p.emit("tick", "Tick.\n");

    let mut decree = p.spawn(&["process"]);
    assert_eq!(p.wait_running(), id);
    let running: Value =
        serde_json::from_str(&fs::read_to_string(p.run_dir(&id).join(".running")).unwrap())
            .unwrap();
    let pid = running["pid"].as_u64().unwrap();

    let status = p.stdout(&["status"], 0);
    let line = format!("      running ticker (invoke of `work`), pid {pid}, for ");
    let detail = status
        .lines()
        .find(|l| l.starts_with(&line))
        .unwrap_or_else(|| panic!("no running line: {status}"));
    assert!(
        detail.ends_with(&format!(", log .decree/runs/{id}/0001-work-ticker.log")),
        "{detail}"
    );
    assert!(
        status.contains(&format!("  active: 1\n    {id}  tick  `work`\n")),
        "{status}"
    );
    let one = p.stdout(&["status", &id], 0);
    assert!(one.contains("  status: active in `work`\n"), "{one}");
    assert!(one.contains(&format!(
        "  running ticker (invoke of `work`), pid {pid}, for "
    )));

    // `decree tail` with no id follows the active run.
    let started = Instant::now();
    let mut tail = p.spawn(&["tail"]);
    let reader = BufReader::new(tail.stdout.take().unwrap());
    let lines: Vec<(Duration, String)> = reader
        .lines()
        .map(|l| (started.elapsed(), l.unwrap()))
        .collect();
    let tail_status = wait_exit(&mut tail, Duration::from_secs(5));
    assert_eq!(tail_status.code(), Some(0));
    let decree_status = wait_exit(&mut decree, Duration::from_secs(5));
    assert_eq!(decree_status.code(), Some(0));

    let text: Vec<&str> = lines.iter().map(|(_, l)| l.as_str()).collect();
    assert_eq!(
        text,
        [
            "== 0001 work/ticker ==",
            "tick 1",
            "tick 2",
            "tick 3",
            "tick 4",
            "tick 5"
        ]
    );
    // Each line arrived as it was written, a second apart, not all at the end.
    let (first, last) = (lines[2].0, lines[5].0);
    assert!(
        last - first >= Duration::from_millis(2500),
        "lines arrived together: {lines:?}"
    );
    // And tail exited once the run finished.
    assert_eq!(p.events(&id).last().unwrap()["type"], "run_finished");
    assert!(started.elapsed() < Duration::from_secs(10));
}

/// `decree tail` exits 1 when there is no such run, or no active run.
#[test]
fn tail_without_a_run_exits_1() {
    let p = Project::init();
    p.decree(&["tail"]).assert().code(1);
    p.decree(&["tail", "no-such-run"]).assert().code(1);
}

/// `decree tail <id>` on a run that waits for a person stops at once, after printing the
/// logs that appear; a finished run's tail prints nothing and exits 0.
#[test]
fn tail_of_a_stopped_run_exits_0() {
    let p = Project::init();
    p.machine("deploy", DEPLOY);
    p.script("ask_person", "#!/usr/bin/env bash\nexit 0\n");
    let id = p.emit("deploy", "Ship v1.\n");
    p.decree(&["process"]).assert().success();
    p.decree(&["tail", &id]).assert().success().stdout("");

    p.machine("flaky", FLAKY);
    p.script("work", FLAKY_WORK);
    fs::write(p.root().join("ok.flag"), "").unwrap();
    let done = p.emit("flaky", "Do the work.\n");
    p.decree(&["process"]).assert().success();
    p.decree(&["tail", &done]).assert().success().stdout("");
}

/// `decree tail` moves on into the child run of a `model` state, and back.
#[test]
fn tail_follows_into_child_runs() {
    let p = Project::init();
    p.machine("triage", TRIAGE);
    p.machine("test_router", TEST_ROUTER);
    p.script(
        "build",
        "#!/usr/bin/env bash\necho built\nsleep 1\necho tested\n",
    );
    p.script(
        "reply",
        &format!("#!/usr/bin/env bash\necho routing\nsleep 1\n{REPLY_LINE}"),
    );
    let id = p.emit("triage", "Build it.\n");

    let mut decree = p.spawn(&["process"]);
    assert_eq!(p.wait_running(), id);
    let tail = p.spawn(&["tail", &id]);
    let out = tail.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        wait_exit(&mut decree, Duration::from_secs(5)).code(),
        Some(0)
    );
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "== 0001 build/build ==\nbuilt\ntested\n== 0001 ask/reply ==\nrouting\n"
    );
}

/// `decree daemon --interval 1s` runs the same pipeline as `process`: a failed inbox run
/// does not stop it, a later message still runs, and SIGTERM stops it with exit 0.
#[test]
fn daemon_runs_messages_through_the_same_pipeline_and_exits_0_on_signal() {
    let p = Project::init();
    p.machine("flaky", FLAKY);
    p.script("work", FLAKY_WORK);
    p.machine(
        "hello",
        &fs::read_to_string("examples/project/.decree/machines/hello.yml").unwrap(),
    );
    p.script("greet", "#!/usr/bin/env bash\necho hello\n");
    let failing = p.emit("flaky", "Fails.\n");

    let mut daemon = p.spawn(&["daemon", "--interval", "1s"]);
    let finished = |id: &str| {
        p.run_dir(id).join("events.jsonl").exists()
            && p.events(id).last().unwrap()["type"] == "run_finished"
    };
    let deadline = Instant::now() + Duration::from_secs(30);
    while !finished(&failing) {
        assert!(Instant::now() < deadline, "the first message never ran");
        thread::sleep(Duration::from_millis(50));
    }
    // Queued while the daemon sleeps: the next pass runs it.
    let id = p.emit("hello", "Say hello.\n");
    while !finished(&id) {
        assert!(Instant::now() < deadline, "the second message never ran");
        thread::sleep(Duration::from_millis(50));
    }
    // SAFETY: sends SIGTERM to the daemon.
    unsafe { libc::kill(daemon.id() as i32, libc::SIGTERM) };
    assert_eq!(
        wait_exit(&mut daemon, Duration::from_secs(15)).code(),
        Some(0)
    );

    let status = p.stdout(&["status"], 0);
    assert!(
        status.contains("  finished: 2\n    done: 1\n") && status.contains("    failed: 1\n"),
        "{status}"
    );
}

/// One duration format (docs/reference/machines.md, Durations): a machine's `timeout`,
/// `prune --older-than` and `daemon --interval` accept and reject the same strings. A bad
/// one fails V16 in a machine and exits 2 on the command line.
#[test]
fn machine_prune_and_daemon_take_the_same_durations() {
    let p = Project::init();
    let machine = |timeout: &str| {
        format!(
            "name: t\ndescription: A script with a time limit.\ninitial: work\nstates:\n  \
             work:\n    invoke: {{ script: {{ name: git_baseline, timeout: '{timeout}' }} }}\n    \
             transitions: {{ done: done }}\n  done: {{ final: true }}\n  failed: {{ final: true }}\n"
        )
    };
    for good in ["90s", "10m", "12h", "7d"] {
        p.machine("t", &machine(good));
        p.decree(&["check"]).assert().code(0);
        p.decree(&["prune", "--older-than", good, "--dry-run"])
            .assert()
            .code(0);
        let mut daemon = p.spawn(&["daemon", "--interval", good]);
        // Kept open until the daemon exits: it prints again on the way out.
        let mut stdout = BufReader::new(daemon.stdout.take().unwrap());
        let mut first = String::new();
        stdout.read_line(&mut first).unwrap();
        assert!(
            first.starts_with("decree daemon: polling every "),
            "{good}: {first}"
        );
        // SAFETY: sends SIGTERM to the daemon.
        unsafe { libc::kill(daemon.id() as i32, libc::SIGTERM) };
        assert_eq!(
            wait_exit(&mut daemon, Duration::from_secs(15)).code(),
            Some(0),
            "{good}"
        );
    }
    for bad in ["1.5h", "1h30m", "10", "-1m", "1w", ""] {
        p.machine("t", &machine(bad));
        let out = p.stdout(&["check"], 1);
        assert!(
            out.contains(&format!("timeout: `{bad}` is not a duration")) && out.contains("(V16)"),
            "{bad}: {out}"
        );
        let flag = |name: &str| format!("--{name}={bad}");
        for args in [
            vec!["prune".to_string(), flag("older-than")],
            vec!["daemon".to_string(), flag("interval")],
        ] {
            let out = p
                .decree(&args.iter().map(String::as_str).collect::<Vec<_>>())
                .output()
                .unwrap();
            assert_eq!(out.status.code(), Some(2), "{args:?}");
            let stderr = String::from_utf8_lossy(&out.stderr);
            assert!(
                stderr.contains(&format!("`{bad}` is not a duration")),
                "{args:?}: {stderr}"
            );
        }
    }
}

/// Project commands outside a project exit non-zero.
#[test]
fn commands_without_a_project_fail() {
    let tmp = TempDir::new().unwrap();
    for cmd in ["process", "check", "graph", "status"] {
        cargo_bin_cmd!("decree")
            .current_dir(tmp.path())
            .arg(cmd)
            .assert()
            .failure();
    }
}

/// `--param` and the trigger reach the script as `DECREE_DATA_*` and `DECREE_TRIGGER`.
#[test]
fn emit_params_and_the_trigger_reach_the_script() {
    let p = Project::init();
    p.machine(
        "hello",
        "name: hello\ndescription: Run one script.\ndata:\n  who: { type: string, default: world }\n\
         initial: greet\nstates:\n  greet:\n    invoke: greet\n    transitions: { done: done }\n  \
         done: { final: true }\n  failed: { final: true }\n",
    );
    p.script(
        "greet",
        "#!/usr/bin/env bash\necho \"hello ${DECREE_DATA_WHO} from ${DECREE_MACHINE}/${DECREE_STATE} \
         trigger=${DECREE_TRIGGER} attempt=${DECREE_ATTEMPT}\"\n",
    );
    let out = p
        .decree(&["emit", "--machine", "hello", "--param", "who=decree"])
        .write_stdin("# Say hello\n")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let id = String::from_utf8(out).unwrap().trim().to_string();
    p.decree(&["process"]).assert().success();
    assert_eq!(
        fs::read_to_string(p.run_dir(&id).join("0001-greet-greet.log")).unwrap(),
        "hello decree from hello/greet trigger=emit attempt=1\n"
    );
}

/// The event a script writes to `$DECREE_EVENT_FILE` picks the transition.
#[test]
fn a_named_event_picks_the_transition() {
    let p = Project::init();
    p.machine(
        "verify",
        "name: verify\ndescription: A script that names its event.\ninitial: verify\nstates:\n  \
         verify:\n    invoke: verify\n    transitions: { pass: passed, fail: failed }\n  \
         passed: { final: true }\n  failed: { final: true }\n",
    );
    fs::create_dir(p.decree_dir().join("scripts/verify")).unwrap();
    p.script(
        "verify/verify.sh",
        "#!/usr/bin/env bash\necho checking\necho pass > \"$DECREE_EVENT_FILE\"\n",
    );
    let id = p.emit("verify", "# Verify\n");
    p.decree(&["process"]).assert().success();
    assert_eq!(
        transitions(&p.events(&id)),
        ["- -> verify (claim)", "verify -> passed (script)"]
    );
}

/// AC: after `decree init`, `.decree/lib/README.md` exists, and a script sees `DECREE_LIB`,
/// the absolute path of `.decree/lib`, and can source a file from it.
#[test]
fn init_creates_lib_and_scripts_see_decree_lib() {
    let p = Project::init();
    let readme = fs::read_to_string(p.decree_dir().join("lib/README.md")).unwrap();
    assert!(readme.contains("never runs anything"), "{readme}");
    fs::write(
        p.decree_dir().join("lib/greeting.sh"),
        "greeting() { echo \"hello from lib\"; }\n",
    )
    .unwrap();
    p.machine(
        "hello",
        &fs::read_to_string("examples/project/.decree/machines/hello.yml").unwrap(),
    );
    p.script(
        "greet",
        "#!/usr/bin/env bash\n. \"$DECREE_LIB/greeting.sh\"\n\
         { echo \"$DECREE_LIB\"; greeting; } > \"$DECREE_PROJECT_ROOT/lib.txt\"\n",
    );
    p.emit("hello", "Say hello.\n");
    p.decree(&["process"]).assert().success();
    let out = fs::read_to_string(p.root().join("lib.txt")).unwrap();
    let lib = p.decree_dir().join("lib");
    assert_eq!(out, format!("{}\nhello from lib\n", lib.display()));
}

/// `decree daemon` reads `.decree/env` again on each pass, so an edit applies without a
/// restart.
#[test]
fn daemon_reads_dotenv_again_on_each_pass() {
    let p = Project::init();
    p.machine(
        "hello",
        &fs::read_to_string("examples/project/.decree/machines/hello.yml").unwrap(),
    );
    p.script(
        "greet",
        "#!/usr/bin/env bash\necho \"$DOTENV_RELOAD_VALUE\" >> \"$DECREE_PROJECT_ROOT/out.txt\"\n",
    );
    let env = p.decree_dir().join("env");
    fs::write(&env, "DOTENV_RELOAD_VALUE=one\n").unwrap();
    let first = p.emit("hello", "First.\n");

    let mut daemon = p.spawn(&["daemon", "--interval", "1s"]);
    let finished = |id: &str| {
        p.run_dir(id).join("events.jsonl").exists()
            && p.events(id).last().unwrap()["type"] == "run_finished"
    };
    let deadline = Instant::now() + Duration::from_secs(30);
    while !finished(&first) {
        assert!(Instant::now() < deadline, "the first message never ran");
        thread::sleep(Duration::from_millis(50));
    }
    fs::write(&env, "DOTENV_RELOAD_VALUE=two\n").unwrap();
    let second = p.emit("hello", "Second.\n");
    while !finished(&second) {
        assert!(Instant::now() < deadline, "the second message never ran");
        thread::sleep(Duration::from_millis(50));
    }
    // SAFETY: sends SIGTERM to the daemon.
    unsafe { libc::kill(daemon.id() as i32, libc::SIGTERM) };
    assert_eq!(
        wait_exit(&mut daemon, Duration::from_secs(15)).code(),
        Some(0)
    );
    let out = fs::read_to_string(p.root().join("out.txt")).unwrap();
    assert_eq!(out, "one\ntwo\n");
}
