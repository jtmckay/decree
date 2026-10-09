//! The recorded runs replayed through the binary (docs/reference/runs.md, Step loop and
//! events.jsonl). For each run in `examples/<name>/.decree/runs/`, or in the fixtures
//! `tests/fixtures/escalation/` and `tests/fixtures/feature/`, that is not a router's child
//! run, a temp project
//! gets a copy of that project's `.decree` whose every script is a
//! stub that replays, execution by execution, the exit code and output recorded for it (a
//! router's script also writes the recorded `reply.json`). The run's message is queued,
//! `decree process` runs, and the events it writes must equal the recorded ones once
//! timestamps, durations, deadlines and generated ids (run ids, and the random trace and span
//! ids) are normalised. The same holds for every child run, its `message.md` (with the
//! `traceparent` a child run gets), its `request.json` and its `traces.jsonl`, whose spans
//! must equal the recorded ones but for their times; and every recorded `reply.json`
//! validates against its request's `reply_schema`.

use assert_cmd::cargo::cargo_bin_cmd;
use assert_cmd::Command;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

mod common;
use common::write_script;

/// Every script in the replay project. Execution `n` of script `<name>` replays
/// `.replay/<name>/<n>/`: `log` (printed to stdout), `exit`,
/// `event` to write to `$DECREE_EVENT_FILE`, and `reply.json` to write to `$DECREE_REPLY`;
/// `sleep` makes it wait to be stopped.
const STUB: &str = r#"#!/usr/bin/env bash
name=$(basename "$0")
dir="$DECREE_PROJECT_ROOT/.replay/${name%%.*}"
n=$(( $(cat "$dir/count" 2>/dev/null || echo 0) + 1 ))
mkdir -p "$dir" && echo "$n" > "$dir/count"
step="$dir/$n"
[ -d "$step" ] || { echo "the project records no execution $n of $name" >&2; exit 99; }
if [ -e "$step/sleep" ]; then
  touch "$DECREE_PROJECT_ROOT/.replay/sleeping"
  exec sleep 100
fi
[ -e "$step/reply.json" ] && cp "$step/reply.json" "$DECREE_REPLY"
[ -e "$step/event" ] && cp "$step/event" "$DECREE_EVENT_FILE"
cat "$step/log"
exit "$(cat "$step/exit")"
"#;

fn repo() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// The projects with recorded runs, relative to the repository: each example that has
/// `.decree/runs/`, the escalation ladder fixture, and the `feature` fixture (nesting, a
/// check, model decisions with router child runs, `emits`).
fn recorded_projects() -> Vec<String> {
    let mut projects: Vec<String> = fs::read_dir(repo().join("examples"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.join(".decree/runs").is_dir())
        .map(|p| format!("examples/{}", p.file_name().unwrap().to_str().unwrap()))
        .collect();
    projects.push("tests/fixtures/escalation".to_string());
    projects.push("tests/fixtures/feature".to_string());
    projects.sort();
    projects
}

fn read_events(run_dir: &Path) -> Vec<Value> {
    fs::read_to_string(run_dir.join("events.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

/// The child runs `events` waits for, in order.
fn children(events: &[Value]) -> Vec<String> {
    events
        .iter()
        .filter(|e| e["type"] == "waiting")
        .filter_map(|e| e["child"].as_str().map(String::from))
        .collect()
}

/// `run` and its child runs, depth first, in the order they ran.
fn run_tree(runs: &Path, run: &str) -> Vec<String> {
    let mut tree = vec![run.to_string()];
    for child in children(&read_events(&runs.join(run))) {
        tree.extend(run_tree(runs, &child));
    }
    tree
}

/// Copy `from` to `to`, recursively, except the files `skip` rejects.
fn copy_dir(from: &Path, to: &Path, skip: &dyn Fn(&Path) -> bool) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let path = entry.unwrap().path();
        if skip(&path) {
            continue;
        }
        let target = to.join(path.file_name().unwrap());
        if path.is_dir() {
            copy_dir(&path, &target, skip);
        } else {
            fs::copy(&path, &target).unwrap();
        }
    }
}

/// Every file under `dir`, recursively.
fn files(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            found.extend(files(&path));
        } else {
            found.push(path);
        }
    }
    found.sort();
    found
}

/// A temp project replaying run `run` of `project` (relative to the repository).
struct Replay {
    tmp: TempDir,
    /// The project's `.decree/`.
    recorded: PathBuf,
    run: String,
}

impl Replay {
    fn new(project: &str, run: &str) -> Replay {
        let tmp = TempDir::new().unwrap();
        let decree = tmp.path().join(".decree");
        let recorded = repo().join(project).join(".decree");
        // The machines, scripts, graphs and cron files; no runs, queue or migrations.
        copy_dir(&recorded, &decree, &|p| {
            p.parent() == Some(recorded.as_path())
                && ["runs", "inbox", "migrations", "processed.md"]
                    .iter()
                    .any(|n| p.file_name().unwrap() == *n)
        });
        for dir in ["runs", "inbox", "migrations"] {
            fs::create_dir_all(decree.join(dir)).unwrap();
        }
        fs::write(decree.join("processed.md"), "").unwrap();
        for script in files(&decree.join("scripts")) {
            write_script(&script, STUB);
        }

        let r = Replay {
            tmp,
            recorded,
            run: run.to_string(),
        };
        r.write_plan();
        r.queue();
        r
    }

    fn recorded_run(&self, id: &str) -> PathBuf {
        self.recorded.join("runs").join(id)
    }

    fn decree(&self) -> PathBuf {
        self.tmp.path().join(".decree")
    }

    /// What each script execution of the run and its child runs must reproduce, from the
    /// recorded `script` events, logs and `reply.json` files; the script an `interrupted`
    /// event names sleeps until it is stopped.
    fn write_plan(&self) {
        let mut count: BTreeMap<String, usize> = BTreeMap::new();
        let mut step = |script: &str| {
            let n = count.entry(script.to_string()).or_default();
            *n += 1;
            let dir = self
                .tmp
                .path()
                .join(".replay")
                .join(script)
                .join(n.to_string());
            fs::create_dir_all(&dir).unwrap();
            dir
        };
        for id in run_tree(&self.recorded.join("runs"), &self.run) {
            let run_dir = self.recorded_run(&id);
            let events = read_events(&run_dir);
            for (i, e) in events.iter().enumerate() {
                let script = e["script"].as_str().unwrap_or_default();
                if e["type"] == "script" {
                    let dir = step(script);
                    if let Some(event) = named_event(e, &events[i + 1..]) {
                        fs::write(dir.join("event"), event).unwrap();
                    }
                    let log = run_dir.join(e["log"].as_str().unwrap());
                    fs::copy(log, dir.join("log")).unwrap();
                    fs::write(dir.join("exit"), e["exit_code"].to_string()).unwrap();
                    let reply = run_dir.join("reply.json");
                    if reply.exists() {
                        fs::copy(reply, dir.join("reply.json")).unwrap();
                    }
                } else if e["type"] == "interrupted" {
                    fs::write(step(script).join("sleep"), "").unwrap();
                }
            }
        }
    }

    /// Queue the run's message as the project received it: a migration from `migrations/`,
    /// anything else in `inbox/` under the claim event's `file`, without the `state` mirror.
    fn queue(&self) {
        let claim = &read_events(&self.recorded_run(&self.run))[0];
        let file = claim["file"].as_str().unwrap();
        if claim["trigger"] == "migration" {
            fs::copy(
                self.recorded.join("migrations").join(file),
                self.decree().join("migrations").join(file),
            )
            .unwrap();
        } else {
            let message =
                fs::read_to_string(self.recorded_run(&self.run).join("message.md")).unwrap();
            let message: String = message
                .lines()
                .filter(|l| !l.starts_with("state: "))
                .map(|l| format!("{l}\n"))
                .collect();
            fs::write(self.decree().join("inbox").join(file), message).unwrap();
        }
    }

    fn cmd(&self, args: &[&str]) -> Command {
        let mut cmd = cargo_bin_cmd!("decree");
        cmd.current_dir(self.tmp.path())
            .env("NO_COLOR", "1")
            .args(args);
        cmd
    }

    fn spawn_process(&self) -> Child {
        std::process::Command::new(env!("CARGO_BIN_EXE_decree"))
            .current_dir(self.tmp.path())
            .env("NO_COLOR", "1")
            .arg("process")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap()
    }

    /// Deliver the reply the recorded run received, through `decree event`, with its body as
    /// the note.
    fn deliver_reply(&self) {
        let received = files(&self.recorded_run(&self.run).join("received"));
        assert_eq!(received.len(), 1, "{received:?}");
        let text = fs::read_to_string(&received[0]).unwrap();
        let (frontmatter, body) = text
            .strip_prefix("---\n")
            .and_then(|t| t.split_once("---\n"))
            .unwrap();
        let key = |k: &str| {
            frontmatter
                .lines()
                .find_map(|l| l.strip_prefix(&format!("{k}: ")))
                .unwrap()
                .to_string()
        };
        let note = body.trim_end_matches('\n');
        self.cmd(&["event", &key("to"), &key("event"), "-m", note])
            .assert()
            .code(0);
    }

    /// Compare the run and its child runs with the recorded ones: events, `message.md` and
    /// `request.json`, under one mapping of generated ids to the recorded ones.
    fn assert_matches_recorded(&self) {
        let recorded_runs = self.recorded.join("runs");
        let runs = self.decree().join("runs");
        let recorded_tree = run_tree(&recorded_runs, &self.run);
        let tree = run_tree(&runs, &self.run);
        assert_eq!(
            tree.len(),
            recorded_tree.len(),
            "{tree:?} vs {recorded_tree:?}"
        );
        let mut ids: BTreeMap<String, String> = tree.iter().cloned().zip(recorded_tree).collect();
        // The reply's filename is the id `decree event` generated.
        let names = |dir: &Path| -> Vec<String> {
            let received = dir.join("received");
            if !received.exists() {
                return Vec::new();
            }
            files(&received)
                .iter()
                .map(|p| p.file_name().unwrap().to_str().unwrap().to_string())
                .collect()
        };
        ids.extend(
            names(&runs.join(&self.run))
                .into_iter()
                .zip(names(&recorded_runs.join(&self.run))),
        );

        // The trace and span ids are random: map each generated one to the recorded one at
        // the same place, so a mismatch anywhere else shows as a difference.
        let mut trace_ids = BTreeMap::new();
        for (id, recorded_id) in &ids {
            let (dir, recorded_dir) = (runs.join(id), recorded_runs.join(recorded_id));
            if !recorded_dir.is_dir() {
                continue;
            }
            for (got, want) in read_events(&dir).iter().zip(read_events(&recorded_dir)) {
                for key in ["trace_id", "span_id", "parent_span_id"] {
                    if let (Some(got), Some(want)) = (got[key].as_str(), want[key].as_str()) {
                        trace_ids.insert(got.to_string(), want.to_string());
                    }
                }
            }
        }
        ids.extend(trace_ids);

        for (id, recorded_id) in &ids {
            let (dir, recorded_dir) = (runs.join(id), recorded_runs.join(recorded_id));
            if !recorded_dir.is_dir() {
                continue;
            }
            let spans = |dir: &Path, ids: &BTreeMap<String, String>| -> Vec<String> {
                fs::read_to_string(dir.join("traces.jsonl"))
                    .unwrap_or_default()
                    .lines()
                    .map(|l| normalise_span(serde_json::from_str(l).unwrap(), ids))
                    .collect()
            };
            assert_eq!(
                spans(&dir, &ids),
                spans(&recorded_dir, &BTreeMap::new()),
                "traces.jsonl of {recorded_id}"
            );
            let produced: Vec<String> = read_events(&dir)
                .into_iter()
                .map(|e| normalise(e, &ids))
                .collect();
            let expected: Vec<String> = read_events(&recorded_dir)
                .into_iter()
                .map(|e| normalise(e, &BTreeMap::new()))
                .collect();
            for (i, (got, want)) in produced.iter().zip(&expected).enumerate() {
                assert_eq!(got, want, "event {} of {recorded_id}", i + 1);
            }
            assert_eq!(produced.len(), expected.len(), "events of {recorded_id}");
            let message = fs::read_to_string(dir.join("message.md")).unwrap();
            let mut message = message;
            for (from, to) in &ids {
                message = message.replace(from.as_str(), to);
            }
            assert_eq!(
                message,
                fs::read_to_string(recorded_dir.join("message.md")).unwrap(),
                "message.md of {recorded_id}"
            );
            let request = recorded_dir.join("request.json");
            if request.exists() {
                let read = |p: &Path| -> Value {
                    serde_json::from_str(&fs::read_to_string(p).unwrap()).unwrap()
                };
                assert_eq!(
                    read(&dir.join("request.json")),
                    read(&request),
                    "request.json of {recorded_id}"
                );
            }
        }
    }
}

/// The event an invoke `script` event's execution named in `$DECREE_EVENT_FILE`, from the
/// `transition` that follows it (after any `onexit` scripts): one with `source: "script"`,
/// whose `invalid_event` is the name when it has one.
fn named_event<'a>(script: &Value, rest: &'a [Value]) -> Option<&'a str> {
    if script["phase"] != "invoke" {
        return None;
    }
    let t = rest.iter().find(|e| e["type"] == "transition")?;
    if t["source"] != "script" || t["from"] != script["state"] {
        return None;
    }
    t.get("invalid_event").unwrap_or(&t["event"]).as_str()
}

/// One event as a JSON line, with timestamps, durations and deadlines replaced by their
/// key, and generated ids (anywhere, `router_error` included) replaced by the recorded ones.
fn normalise(mut event: Value, ids: &BTreeMap<String, String>) -> String {
    let map = event.as_object_mut().unwrap();
    for key in ["ts", "started_at", "duration_ms", "timeout_at"] {
        if let Some(v) = map.get_mut(key) {
            if !v.is_null() {
                *v = Value::String(format!("<{key}>"));
            }
        }
    }
    let mut line = event.to_string();
    for (id, recorded_id) in ids {
        line = line.replace(id.as_str(), recorded_id);
    }
    line
}

/// One `traces.jsonl` line as JSON, with decree's version replaced by `<version>`, each span's
/// start and end times by their key, and generated ids by the recorded ones.
fn normalise_span(mut line: Value, ids: &BTreeMap<String, String>) -> String {
    // The recording was made by another decree version.
    let resource = &mut line["resourceSpans"][0];
    for attribute in resource["resource"]["attributes"].as_array_mut().unwrap() {
        if attribute["key"] == "service.version" {
            attribute["value"]["stringValue"] = Value::String("<version>".into());
        }
    }
    resource["scopeSpans"][0]["scope"]["version"] = Value::String("<version>".into());
    for span in line["resourceSpans"][0]["scopeSpans"][0]["spans"]
        .as_array_mut()
        .unwrap()
    {
        for key in ["startTimeUnixNano", "endTimeUnixNano"] {
            span[key] = Value::String(format!("<{key}>"));
        }
    }
    let mut line = line.to_string();
    for (id, recorded_id) in ids {
        line = line.replace(id.as_str(), recorded_id);
    }
    line
}

#[test]
fn project_migration_01_tries_claude_after_local_and_finishes_done_as_recorded() {
    let r = Replay::new("examples/project", "01-rate-limit-upload");
    let process = r.cmd(&["process"]).assert();
    r.assert_matches_recorded();
    process.code(0);
    assert_eq!(
        fs::read_to_string(r.decree().join("processed.md")).unwrap(),
        "01-rate-limit-upload.md\n"
    );
}

#[test]
fn project_deploy_run_waits_for_a_person_as_recorded() {
    let r = Replay::new("examples/project", "20261001T153012Z-7b4e2a");
    let process = r.cmd(&["process"]).assert();
    r.assert_matches_recorded();
    let stdout = String::from_utf8_lossy(&process.code(0).get_output().stdout).into_owned();
    let run = fs::read_dir(r.decree().join("runs"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .file_name();
    let run = run.to_str().unwrap();
    assert!(
        stdout.contains(&format!("decree event {run}.w3 approve")),
        "{stdout}"
    );
}

#[test]
fn project_cron_run_interrupted_by_sigint_as_recorded() {
    let r = Replay::new("examples/project", "20261001T030000Z-c4e81b");
    let mut decree = r.spawn_process();
    let sleeping = r.tmp.path().join(".replay/sleeping");
    let deadline = Instant::now() + Duration::from_secs(30);
    while !sleeping.exists() {
        assert!(Instant::now() < deadline, "the script never started");
        thread::sleep(Duration::from_millis(5));
    }
    // SAFETY: sends SIGINT to the decree process.
    unsafe { libc::kill(decree.id() as i32, libc::SIGINT) };
    assert_eq!(decree.wait().unwrap().code(), Some(130));
    r.assert_matches_recorded();
}

#[test]
fn feature_migration_01_finishes_done_as_recorded() {
    let r = Replay::new("tests/fixtures/feature", "01-rate-limit-upload");
    let process = r.cmd(&["process"]).assert();
    r.assert_matches_recorded();
    process.code(0);
    assert_eq!(
        fs::read_to_string(r.decree().join("processed.md")).unwrap(),
        "01-rate-limit-upload.md\n"
    );
}

#[test]
fn feature_migration_02_waits_for_a_person_as_recorded() {
    let r = Replay::new("tests/fixtures/feature", "02-upload-quota-per-plan");
    let process = r.cmd(&["process"]).assert();
    r.assert_matches_recorded();
    let stdout = String::from_utf8_lossy(&process.code(0).get_output().stdout).into_owned();
    assert!(
        stdout.contains("decree event 02-upload-quota-per-plan.w15 retry"),
        "{stdout}"
    );
}

#[test]
fn feature_triage_run_picks_small_change_as_recorded() {
    let r = Replay::new("tests/fixtures/feature", "20261001T151455Z-5d2e90");
    let process = r.cmd(&["process"]).assert();
    r.assert_matches_recorded();
    process.code(0);
}

#[test]
fn escalation_run_climbs_to_a_person_and_files_the_reply_as_recorded() {
    let r = Replay::new("tests/fixtures/escalation", "20261001T170412Z-3f9a51");
    let waiting = r.cmd(&["process"]).assert();
    r.deliver_reply();
    let process = r.cmd(&["process"]).assert();
    r.assert_matches_recorded();
    waiting.code(0);
    process.code(0);
}

/// Every recorded run in every recorded project that is not a router's child run has a test
/// above.
#[test]
fn every_recorded_run_that_is_not_a_child_run_is_replayed() {
    let source = fs::read_to_string(file!()).unwrap_or_else(|_| {
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(file!())).unwrap()
    });
    let mut top: Vec<String> = Vec::new();
    for name in recorded_projects() {
        for entry in fs::read_dir(repo().join(&name).join(".decree/runs")).unwrap() {
            let path = entry.unwrap().path();
            let message = fs::read_to_string(path.join("message.md")).unwrap();
            if !message.contains("\ntrigger: invoke\n") {
                let id = path.file_name().unwrap().to_str().unwrap();
                top.push(format!("Replay::new(\"{name}\", \"{id}\")"));
            }
        }
    }
    top.sort();
    assert_eq!(top.len(), 7, "{top:?}");
    for call in top {
        assert!(source.contains(&call), "no test calls {call}");
    }
}

/// Every recorded `request.json` has a `reply_schema` whose `event` enum is the request's
/// options, and the router's recorded `reply.json` validates against it.
#[test]
fn every_recorded_reply_validates_against_its_request_reply_schema() {
    let mut checked = 0;
    for project in recorded_projects() {
        let runs = repo().join(project).join(".decree/runs");
        for request in files(&runs)
            .into_iter()
            .filter(|p| p.ends_with("request.json"))
        {
            let read = |p: &Path| -> Value {
                serde_json::from_str(&fs::read_to_string(p).unwrap()).unwrap()
            };
            let req = read(&request);
            let options: Vec<&Value> = req["options"]
                .as_array()
                .unwrap()
                .iter()
                .map(|o| &o["event"])
                .collect();
            let schema = &req["reply_schema"];
            let events: Vec<&Value> = schema["properties"]["event"]["enum"]
                .as_array()
                .unwrap_or_else(|| panic!("{}: no reply_schema", request.display()))
                .iter()
                .collect();
            assert_eq!(events, options, "{}", request.display());
            let validator = jsonschema::draft202012::new(schema).unwrap();
            let reply = read(&request.with_file_name("reply.json"));
            assert!(validator.is_valid(&reply), "{}: {reply}", request.display());
            checked += 1;
        }
    }
    assert_eq!(checked, 5);
}
