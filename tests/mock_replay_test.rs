//! The mock's runs replayed through the binary (docs/reference/runs.md, Step loop and
//! events.jsonl). For each run in `mock/.decree/runs/` that is not a router's child run, a
//! temp project gets a copy of `mock/.decree` whose every script is a stub that replays,
//! execution by execution, the exit code and output the mock recorded for it (a router's
//! script also writes the recorded `reply.json`). The run's message is queued, `decree
//! process` runs, and the events it writes must equal the mock's once timestamps,
//! durations, deadlines and generated ids are normalised. The same holds for every child
//! run, its `message.md` and its `request.json`.

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
/// `.replay/<name>/<n>/`: `log` (stdout lines, and `[stderr] ` lines to stderr), `exit`,
/// and `reply.json` to write to `$DECREE_REPLY`; `sleep` makes it wait to be stopped.
const STUB: &str = r#"#!/usr/bin/env bash
name=$(basename "$0")
dir="$DECREE_PROJECT_ROOT/.replay/${name%%.*}"
n=$(( $(cat "$dir/count" 2>/dev/null || echo 0) + 1 ))
mkdir -p "$dir" && echo "$n" > "$dir/count"
step="$dir/$n"
[ -d "$step" ] || { echo "the mock records no execution $n of $name" >&2; exit 99; }
if [ -e "$step/sleep" ]; then
  touch "$DECREE_PROJECT_ROOT/.replay/sleeping"
  exec sleep 100
fi
[ -e "$step/reply.json" ] && cp "$step/reply.json" "$DECREE_REPLY"
while IFS= read -r line || [ -n "$line" ]; do
  case "$line" in
    '[stderr] '*) printf '%s\n' "${line#'[stderr] '}" >&2 ;;
    *) printf '%s\n' "$line" ;;
  esac
done < "$step/log"
exit "$(cat "$step/exit")"
"#;

fn mock() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("mock/.decree")
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

/// A temp project replaying mock run `run`.
struct Replay {
    tmp: TempDir,
    run: String,
}

impl Replay {
    fn new(run: &str) -> Replay {
        let tmp = TempDir::new().unwrap();
        let decree = tmp.path().join(".decree");
        let mock = mock();
        // The machines, scripts, graphs and cron files; no runs, queue or migrations.
        copy_dir(&mock, &decree, &|p| {
            p.parent() == Some(mock.as_path())
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
            run: run.to_string(),
        };
        r.write_plan();
        r.queue();
        r
    }

    fn mock_run(&self, id: &str) -> PathBuf {
        mock().join("runs").join(id)
    }

    fn decree(&self) -> PathBuf {
        self.tmp.path().join(".decree")
    }

    /// What each script execution of the run and its child runs must reproduce, from the
    /// mock's `script` events, logs and `reply.json` files; the script an `interrupted`
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
        for id in run_tree(&mock().join("runs"), &self.run) {
            let run_dir = self.mock_run(&id);
            for e in read_events(&run_dir) {
                let script = e["script"].as_str().unwrap_or_default();
                if e["type"] == "script" {
                    let dir = step(script);
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

    /// Queue the run's message as the mock received it: a migration from `migrations/`,
    /// anything else in `inbox/` under the claim event's `file`, without the `state` mirror.
    fn queue(&self) {
        let claim = &read_events(&self.mock_run(&self.run))[0];
        let file = claim["file"].as_str().unwrap();
        if claim["trigger"] == "migration" {
            fs::copy(
                mock().join("migrations").join(file),
                self.decree().join("migrations").join(file),
            )
            .unwrap();
        } else {
            let message = fs::read_to_string(self.mock_run(&self.run).join("message.md")).unwrap();
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

    /// Deliver the reply the mock run received, through `decree event`, with its body as
    /// the note.
    fn deliver_reply(&self) {
        let received = files(&self.mock_run(&self.run).join("received"));
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

    /// Compare the run and its child runs with the mock's: events, `message.md` and
    /// `request.json`, under one mapping of generated ids to the mock's.
    fn assert_matches_mock(&self) {
        let mock_runs = mock().join("runs");
        let runs = self.decree().join("runs");
        let mock_tree = run_tree(&mock_runs, &self.run);
        let tree = run_tree(&runs, &self.run);
        assert_eq!(tree.len(), mock_tree.len(), "{tree:?} vs {mock_tree:?}");
        let mut ids: BTreeMap<String, String> = tree.iter().cloned().zip(mock_tree).collect();
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
                .zip(names(&mock_runs.join(&self.run))),
        );

        for (id, mock_id) in &ids {
            let (dir, mock_dir) = (runs.join(id), mock_runs.join(mock_id));
            if !mock_dir.is_dir() {
                continue;
            }
            let produced: Vec<String> = read_events(&dir)
                .into_iter()
                .map(|e| normalise(e, &ids))
                .collect();
            let expected: Vec<String> = read_events(&mock_dir)
                .into_iter()
                .map(|e| normalise(e, &BTreeMap::new()))
                .collect();
            for (i, (got, want)) in produced.iter().zip(&expected).enumerate() {
                assert_eq!(got, want, "event {} of {mock_id}", i + 1);
            }
            assert_eq!(produced.len(), expected.len(), "events of {mock_id}");
            let message = fs::read_to_string(dir.join("message.md")).unwrap();
            let mut message = message;
            for (from, to) in &ids {
                message = message.replace(from.as_str(), to);
            }
            assert_eq!(
                message,
                fs::read_to_string(mock_dir.join("message.md")).unwrap(),
                "message.md of {mock_id}"
            );
            let request = mock_dir.join("request.json");
            if request.exists() {
                let read = |p: &Path| -> Value {
                    serde_json::from_str(&fs::read_to_string(p).unwrap()).unwrap()
                };
                assert_eq!(
                    read(&dir.join("request.json")),
                    read(&request),
                    "request.json of {mock_id}"
                );
            }
        }
    }
}

/// One event as a JSON line, with timestamps, durations and deadlines replaced by their
/// key, and generated ids (anywhere, `router_error` included) replaced by the mock's.
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
    for (id, mock_id) in ids {
        line = line.replace(id.as_str(), mock_id);
    }
    line
}

#[test]
fn migration_01_finishes_done_as_in_the_mock() {
    let r = Replay::new("01-rate-limit-upload");
    let process = r.cmd(&["process"]).assert();
    r.assert_matches_mock();
    process.code(0);
    assert_eq!(
        fs::read_to_string(r.decree().join("processed.md")).unwrap(),
        "01-rate-limit-upload.md\n"
    );
}

#[test]
fn migration_02_waits_for_a_person_as_in_the_mock() {
    let r = Replay::new("02-upload-quota-per-plan");
    let process = r.cmd(&["process"]).assert();
    r.assert_matches_mock();
    let stdout = String::from_utf8_lossy(&process.code(0).get_output().stdout).into_owned();
    assert!(
        stdout.contains("decree event 02-upload-quota-per-plan.w15 retry"),
        "{stdout}"
    );
}

#[test]
fn triage_run_picks_small_change_as_in_the_mock() {
    let r = Replay::new("20261001T151455Z-5d2e90");
    let process = r.cmd(&["process"]).assert();
    r.assert_matches_mock();
    process.code(0);
}

#[test]
fn sort_document_run_climbs_to_a_person_and_files_the_reply_as_in_the_mock() {
    let r = Replay::new("20261001T170412Z-3f9a51");
    let waiting = r.cmd(&["process"]).assert();
    r.deliver_reply();
    let process = r.cmd(&["process"]).assert();
    r.assert_matches_mock();
    waiting.code(0);
    process.code(0);
}

#[test]
fn cron_run_interrupted_by_sigint_as_in_the_mock() {
    let r = Replay::new("20261001T030000Z-c4e81b");
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
    r.assert_matches_mock();
}

/// Every run in the mock that is not a router's child run has a test above.
#[test]
fn every_mock_run_that_is_not_a_child_run_is_replayed() {
    let source = fs::read_to_string(file!()).unwrap_or_else(|_| {
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(file!())).unwrap()
    });
    let mut top: Vec<String> = Vec::new();
    for entry in fs::read_dir(mock().join("runs")).unwrap() {
        let path = entry.unwrap().path();
        let message = fs::read_to_string(path.join("message.md")).unwrap();
        if !message.contains("\ntrigger: invoke\n") {
            top.push(path.file_name().unwrap().to_str().unwrap().to_string());
        }
    }
    top.sort();
    assert_eq!(top.len(), 5, "{top:?}");
    for id in top {
        assert!(
            source.contains(&format!("Replay::new(\"{id}\")")),
            "mock run {id} is not replayed"
        );
    }
}
