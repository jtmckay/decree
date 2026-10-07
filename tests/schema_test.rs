//! The JSON Schemas (docs/reference/README.md, Schemas): each is a valid draft 2020-12
//! schema with a description on every property; every machine in `examples/`,
//! `tests/fixtures/escalation/`, `tests/fixtures/feature/`, `src/templates/`, this repository and a fresh `decree init`
//! for each `--ai` validates against `machine.schema.json`; every message in those projects
//! against `message.schema.json`; every `events.jsonl` line, `request.json` and `reply.json`
//! in their recorded runs and in a run made here against `events.schema.json`, `request.schema.json`
//! and `reply.schema.json`; `decree schema` writes them all to `.decree/schema/v1/` and
//! removes anything else, and `decree check` warns until it has. Whether the schemas reject
//! what `decree check` rejects is tested case by case in `validation_test.rs`; every line
//! the property test in `interpreter_props.rs` writes is validated there.

use assert_cmd::cargo::cargo_bin_cmd;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

mod common;
#[path = "common/schema.rs"]
mod schema;
#[path = "common/traces.rs"]
mod traces;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The files in `dir` (not recursive) whose name ends with `suffix`, sorted; none if `dir`
/// does not exist.
fn files_in(dir: &Path, suffix: &str) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_file())
        .filter(|p| {
            let name = p.file_name().unwrap().to_string_lossy();
            name.ends_with(suffix) && !name.starts_with('.')
        })
        .collect();
    out.sort();
    out
}

/// The example projects: each of `examples/`, and the fixtures whose recorded runs
/// `replay_test.rs` replays: the escalation ladder in `tests/fixtures/escalation/` and the
/// `feature` project in `tests/fixtures/feature/`.
fn example_projects() -> Vec<PathBuf> {
    let mut projects: Vec<PathBuf> = fs::read_dir(repo().join("examples"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    projects.push(repo().join("tests/fixtures/escalation"));
    projects.push(repo().join("tests/fixtures/feature"));
    projects.sort();
    projects
}

/// The machine files of every project in this repository: the example projects, and the
/// repository's own `.decree/`.
fn project_machines() -> Vec<PathBuf> {
    let mut projects = vec![repo()];
    projects.extend(example_projects());
    let machines: Vec<PathBuf> = projects
        .iter()
        .flat_map(|p| files_in(&p.join(".decree/machines"), ".yml"))
        .collect();
    assert!(machines.len() > 15, "found only {machines:?}");
    machines
}

/// One line per file in `files` that `validate` rejects or cannot read, naming the file.
fn rejected(files: &[(String, String)], validate: impl Fn(&str) -> Option<Vec<String>>) -> String {
    let mut out = Vec::new();
    for (name, text) in files {
        match validate(text) {
            None => out.push(format!("{name}: does not parse")),
            Some(errors) if !errors.is_empty() => {
                out.push(format!("{name}:\n  {}", errors.join("\n  ")))
            }
            Some(_) => {}
        }
    }
    out.join("\n")
}

fn read(paths: &[PathBuf]) -> Vec<(String, String)> {
    paths
        .iter()
        .map(|p| {
            let name = p.strip_prefix(repo()).unwrap_or(p).display().to_string();
            (name, fs::read_to_string(p).unwrap())
        })
        .collect()
}

fn machine_rejects(files: &[(String, String)]) -> String {
    let validator = schema::machine_validator();
    rejected(files, |text| schema::machine_errors(&validator, text))
}

#[test]
fn every_schema_is_a_valid_draft_2020_12_schema() {
    for (name, text) in schema::ALL {
        let schema: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(
            schema["$schema"], "https://json-schema.org/draft/2020-12/schema",
            "{name}"
        );
        if let Err(e) = jsonschema::draft202012::meta::validate(&schema) {
            panic!("{name}.schema.json is not a valid draft 2020-12 schema: {e}");
        }
        assert!(
            schema["$id"].is_string() && schema["title"].is_string(),
            "{name}"
        );
    }
}

/// Subschemas that only narrow keys declared elsewhere: conditional branches, and the defs
/// they reference. Their properties need no `description` of their own.
const NARROWING: &[&str] = &["if", "then", "else", "oneOf", "allOf", "not"];
/// Keys whose values are instances, not subschemas.
const INSTANCES: &[&str] = &["examples", "const", "enum", "default"];
const NARROWING_DEFS: &[&str] = &[
    "finalState",
    "compoundState",
    "atomicState",
    "options",
    "option",
];

/// Every property a schema declares has a `description`, its own or its `$ref`'s, so
/// an editor shows it on hover and a model reading the schema learns what each key means.
#[test]
fn every_property_has_a_description() {
    fn walk(root: &serde_json::Value, at: &str, value: &serde_json::Value, out: &mut Vec<String>) {
        let Some(map) = value.as_object() else {
            return;
        };
        for (key, prop) in map
            .get("properties")
            .and_then(|p| p.as_object())
            .into_iter()
            .flatten()
        {
            let target = prop
                .get("$ref")
                .and_then(|r| r.as_str())
                .and_then(|r| r.strip_prefix('#'))
                .and_then(|pointer| root.pointer(pointer));
            let described = prop.is_boolean()
                || prop.get("description").is_some()
                || target.is_some_and(|t| t.get("description").is_some());
            if !described {
                out.push(format!("{at}/properties/{key}"));
            }
        }
        for (key, child) in map {
            if NARROWING.contains(&key.as_str()) || INSTANCES.contains(&key.as_str()) {
                continue;
            }
            let children: Vec<(String, &serde_json::Value)> = match (key.as_str(), child) {
                ("$defs", serde_json::Value::Object(defs)) => defs
                    .iter()
                    .filter(|(name, _)| !NARROWING_DEFS.contains(&name.as_str()))
                    .map(|(name, def)| (format!("$defs/{name}"), def))
                    .collect(),
                (_, serde_json::Value::Array(items)) => items
                    .iter()
                    .enumerate()
                    .map(|(i, item)| (format!("{key}/{i}"), item))
                    .collect(),
                _ => vec![(key.clone(), child)],
            };
            for (path, child) in children {
                walk(root, &format!("{at}/{path}"), child, out);
            }
        }
    }
    for (name, text) in schema::ALL {
        let schema: serde_json::Value = serde_json::from_str(text).unwrap();
        let mut missing = Vec::new();
        walk(&schema, "#", &schema, &mut missing);
        assert!(missing.is_empty(), "{name}: no description at {missing:?}");
    }
}

#[test]
fn every_machine_in_the_repository_validates() {
    let found = machine_rejects(&read(&project_machines()));
    assert!(found.is_empty(), "{found}");
}

/// The machine templates, with their placeholders filled as `decree init --ai claude` fills
/// them.
#[test]
fn every_machine_template_validates() {
    let templates = repo().join("src/templates");
    let mut paths = files_in(&templates.join("machines"), ".yml");
    paths.push(templates.join("router/router.yml"));
    let files: Vec<(String, String)> = read(&paths)
        .into_iter()
        .map(|(name, text)| {
            let filled = text
                .replace("{ai_title}", "Claude")
                .replace("{ai_cli}", "claude -p")
                .replace("{ai}", "claude");
            assert!(!filled.contains("{ai"), "{name}: unfilled placeholder");
            (name, filled)
        })
        .collect();
    assert_eq!(files.len(), 3);
    let found = machine_rejects(&files);
    assert!(found.is_empty(), "{found}");
}

#[test]
fn every_machine_of_a_fresh_init_validates_for_each_ai() {
    for ai in ["claude", "copilot", "opencode"] {
        let tmp = TempDir::new().unwrap();
        cargo_bin_cmd!("decree")
            .current_dir(tmp.path())
            .env("NO_COLOR", "1")
            .args(["init", "--ai", ai])
            .assert()
            .success();
        let paths = files_in(&tmp.path().join(".decree/machines"), ".yml");
        assert_eq!(paths.len(), 3, "{ai}");
        for path in &paths {
            let text = fs::read_to_string(path).unwrap();
            assert!(
                text.starts_with(
                    "# yaml-language-server: $schema=../schema/v1/machine.schema.json\n# Graph: "
                ),
                "{ai}: {}",
                path.display()
            );
        }
        let found = machine_rejects(&read(&paths));
        assert!(found.is_empty(), "{ai}: {found}");
    }
}

/// Migrations, inbox messages, cron files, and the messages and replies in the recorded runs
/// of every example project.
#[test]
fn every_message_in_examples_validates() {
    let mut paths = Vec::new();
    for project in example_projects() {
        let decree = project.join(".decree");
        for dir in ["migrations", "inbox", "cron"] {
            paths.extend(files_in(&decree.join(dir), ".md"));
        }
        let Ok(runs) = fs::read_dir(decree.join("runs")) else {
            continue;
        };
        for run in runs {
            let run = run.unwrap().path();
            paths.extend(files_in(&run, "message.md"));
            paths.extend(files_in(&run.join("received"), ".md"));
        }
    }
    let files = read(&paths);
    for (what, part) in [
        ("reply", "/received/"),
        ("inbox message", "/inbox/"),
        ("cron file", "/cron/"),
        ("migration", "/migrations/"),
        ("run", "/runs/"),
    ] {
        assert!(
            files.iter().any(|(n, _)| n.contains(part)),
            "no {what} in examples/"
        );
    }
    let validator = schema::message_validator();
    let found = rejected(&files, |text| schema::message_errors(&validator, text));
    assert!(found.is_empty(), "{found}");
}

/// A machine with every `invoke` kind, long and short.
const EVERY_INVOKE: &str = "\
name: m
description: Every invoke kind.
data:
  max_rounds: { type: int, default: 2 }
  file: { type: string, default: a.pdf }
initial: short
states:
  short:
    invoke: work
    transitions: { done: long }
  long:
    invoke:
      script: { name: work, attempts: 2, timeout: 60s }
    transitions: { done: named }
  named:
    invoke:
      script: work
    transitions: { done: rounds }
  rounds:
    invoke:
      check: { visits: long, less_than: { data: max_rounds } }
    transitions: { true: pdf, false: done }
  pdf:
    invoke:
      check: { data: file, matches: '\\.pdf$' }
    transitions: { true: text, false: done }
  text:
    invoke:
      check: { output: long, matches: '(?i)invoice' }
    transitions: { true: pick, false: done }
  pick:
    invoke:
      model:
        question: Which way?
        router: router
        min_confidence: 0.8
        output: long
    transitions:
      left:   { target: sure, description: Go left. }
      right:  { target: ask, description: Go right. }
      unsure: ask
  sure:
    invoke:
      check: { confidence: pick, at_least: 0.4 }
    transitions: { true: ask, false: child }
  ask:
    invoke:
      person:
        question: Ship it?
        ask: ask_person
        timeout: 1d
    transitions:
      approve: { target: child, description: Ship it. }
      reject:  { target: done, description: Do not ship. }
  child:
    invoke: { machine: hello }
    transitions: { done: params }
  params:
    invoke:
      machine: { name: hello, params: { label: release, rounds: 3, strict: true } }
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
";

#[test]
fn every_invoke_kind_and_short_form_validates() {
    let found = machine_rejects(&[("every invoke".into(), EVERY_INVOKE.into())]);
    assert!(found.is_empty(), "{found}");
}

/// `true:` and `false:` unquoted are YAML booleans; as JSON keys they are "true" and
/// "false", the events of a `check`, as decree reads them.
#[test]
fn boolean_transition_keys_are_the_events_true_and_false() {
    let json = schema::yaml_to_json("transitions: { true: a, false: b }").unwrap();
    assert_eq!(
        json,
        serde_json::json!({ "transitions": { "true": "a", "false": "b" } })
    );
    let unquoted = EVERY_INVOKE.replace("true: pdf, false: done", "'true': pdf, \"false\": done");
    let found = machine_rejects(&[("quoted".into(), unquoted)]);
    assert!(found.is_empty(), "{found}");
}

/// `attempts` as a list of values, model ids among them, is accepted as the count is.
#[test]
fn attempts_list_is_accepted() {
    let validator = schema::machine_validator();
    let text = EVERY_INVOKE.replacen(
        "attempts: 2,",
        "attempts: [local, qwen3:8b, claude-opus-5-5, local],",
        1,
    );
    assert!(text != EVERY_INVOKE);
    let errors = schema::machine_errors(&validator, &text).unwrap();
    assert!(errors.is_empty(), "{errors:?}");
}

/// Mistakes an editor underlines: each is rejected at the key or value that is wrong.
#[test]
fn mistakes_are_rejected_where_they_are() {
    let validator = schema::machine_validator();
    for (from, to, at) in [
        ("    invoke: work\n", "    invok: work\n", "/states/short"),
        (
            "{ name: work, attempts: 2,",
            "{ name: work, max_atempts: 2,",
            "/states/long/invoke",
        ),
        (
            "{ name: work, attempts: 2,",
            "{ name: work, max_attempts: 2,",
            "/states/long/invoke",
        ),
        (
            "{ name: work, attempts: 2,",
            "{ name: work, attempts: 0,",
            "/states/long/invoke",
        ),
        (
            "{ name: work, attempts: 2,",
            "{ name: work, attempts: [],",
            "/states/long/invoke",
        ),
        (
            "{ name: work, attempts: 2,",
            "{ name: work, attempts: [local, \"claude opus\"],",
            "/states/long/invoke",
        ),
        (
            "min_confidence: 0.8",
            "min_confidence: 80",
            "/states/pick/invoke",
        ),
        ("at_least: 0.4", "at_least: high", "/states/sure/invoke"),
        (
            "less_than: { data: max_rounds }",
            "less_than: 1.5",
            "/states/rounds/invoke",
        ),
        (
            "{ true: pdf, false: done }",
            "{ yes: pdf, no: done }",
            "/states/rounds/transitions",
        ),
        (
            "Go left. }",
            "Go left. }\n      done:   { target: sure, description: Done. }",
            "/states/pick/transitions/done",
        ),
        (
            "{ target: child, description: Ship it. }",
            "child",
            "/states/ask/transitions/approve",
        ),
        (
            "{ done: long }",
            "{ Done: long }",
            "/states/short/transitions",
        ),
        (
            "  done: { final: true }\n",
            "  done: { final: true, onexit: [notify] }\n",
            "/states/done",
        ),
        ("  failed: { final: true }\n", "", "/states"),
        (
            "{ type: int, default: 2 }",
            "{ type: int, default: two }",
            "/data/max_rounds/default",
        ),
    ] {
        assert!(EVERY_INVOKE.contains(from), "{from}");
        let text = EVERY_INVOKE.replacen(from, to, 1);
        let errors = schema::machine_errors(&validator, &text).unwrap();
        assert!(
            errors
                .iter()
                .any(|e| e.ends_with(&format!("(at {at})")) || e.contains(&format!("(at {at}/"))),
            "{to}: expected an error at {at}, got {errors:?}"
        );
    }
}

/// A project with a machine and no `.decree/schema/`.
fn project() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let decree = tmp.path().join(".decree");
    fs::create_dir_all(decree.join("machines")).unwrap();
    fs::write(
        decree.join("machines/hello.yml"),
        fs::read_to_string(repo().join("examples/project/.decree/machines/hello.yml")).unwrap(),
    )
    .unwrap();
    common_script(&decree.join("scripts/greet"));
    tmp
}

fn common_script(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, "#!/usr/bin/env bash\nexit 0\n").unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// `decree <cmd>` in `dir`: (exit code, stdout, stderr).
fn decree(dir: &Path, cmd: &str) -> (i32, String, String) {
    let out = cargo_bin_cmd!("decree")
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .arg(cmd)
        .output()
        .unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

#[test]
fn decree_schema_writes_every_schema_in_v1() {
    let tmp = project();
    let (code, stdout, _) = decree(tmp.path(), "schema");
    let expected: String = schema::ALL
        .iter()
        .map(|(name, _)| format!(".decree/schema/v1/{name}.schema.json\n"))
        .collect();
    assert_eq!((code, stdout.as_str()), (0, expected.as_str()));
    let dir = tmp.path().join(".decree/schema/v1");
    for (name, text) in schema::ALL {
        assert_eq!(
            fs::read_to_string(dir.join(format!("{name}.schema.json"))).unwrap(),
            text,
            "{name}"
        );
    }
    // Every file in v1/ and v1/cli/, temp files included.
    let names: Vec<String> = [dir.clone(), dir.join("cli")]
        .iter()
        .flat_map(|d| fs::read_dir(d).unwrap())
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_file())
        .map(|p| p.display().to_string())
        .collect();
    assert_eq!(
        names.len(),
        schema::ALL.len(),
        "a temp file was left: {names:?}"
    );
}

#[test]
fn check_warns_when_the_schema_is_missing_or_stale_until_decree_schema_runs() {
    let tmp = project();
    decree(tmp.path(), "graph");
    let (code, stdout, stderr) = decree(tmp.path(), "check");
    assert_eq!((code, stdout.as_str()), (0, ""));
    let missing: String = schema::ALL
        .iter()
        .map(|(name, _)| {
            format!("warning: schema/v1/{name}.schema.json: missing; run `decree schema`\n")
        })
        .collect();
    assert_eq!(stderr, missing);

    decree(tmp.path(), "schema");
    let dir = tmp.path().join(".decree/schema/v1");
    fs::write(dir.join("machine.schema.json"), "{}\n").unwrap();
    let (code, stdout, stderr) = decree(tmp.path(), "check");
    assert_eq!((code, stdout.as_str()), (0, ""));
    assert_eq!(
        stderr,
        "warning: schema/v1/machine.schema.json: out of date; run `decree schema`\n"
    );

    decree(tmp.path(), "schema");
    assert_eq!(
        decree(tmp.path(), "check"),
        (0, String::new(), String::new())
    );
}

/// A project from before the schemas were versioned: `decree check` warns about the
/// unversioned files, and `decree schema` leaves only `.decree/schema/v1/`.
#[test]
fn unversioned_schemas_are_reported_then_removed() {
    let tmp = project();
    decree(tmp.path(), "graph");
    decree(tmp.path(), "schema");
    let dir = tmp.path().join(".decree/schema");
    for name in ["machine.schema.json", "message.schema.json"] {
        fs::write(dir.join(name), "{}\n").unwrap();
    }
    let (code, stdout, stderr) = decree(tmp.path(), "check");
    assert_eq!((code, stdout.as_str()), (0, ""));
    assert_eq!(
        stderr,
        "warning: schema/machine.schema.json: not one decree writes; run `decree schema`\n\
         warning: schema/message.schema.json: not one decree writes; run `decree schema`\n"
    );

    decree(tmp.path(), "schema");
    let names: Vec<String> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["v1"]);
    assert_eq!(
        decree(tmp.path(), "check"),
        (0, String::new(), String::new())
    );
}

/// Every file named `name` in the recorded runs of every example project.
fn recorded(name: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for project in example_projects() {
        let Ok(runs) = fs::read_dir(project.join(".decree/runs")) else {
            continue;
        };
        for run in runs {
            let path = run.unwrap().path().join(name);
            if path.is_file() {
                paths.push(path);
            }
        }
    }
    paths.sort();
    paths
}

fn events_rejects(files: &[(String, String)]) -> String {
    let validator = schema::events_validator();
    rejected(files, |text| Some(schema::events_errors(&validator, text)))
}

fn json_rejects(schema_text: &str, files: &[(String, String)]) -> String {
    let validator = schema::validator(schema_text);
    rejected(files, |text| schema::json_errors(&validator, text))
}

#[test]
fn every_event_in_examples_validates() {
    let paths = recorded("events.jsonl");
    assert!(paths.len() >= 10, "found only {paths:?}");
    let found = events_rejects(&read(&paths));
    assert!(found.is_empty(), "{found}");
}

/// Every recorded run's `traces.jsonl` is OTLP/JSON and agrees with its `events.jsonl`, and
/// a child run shares its parent's trace id (docs/reference/observability.md, Traces).
#[test]
fn every_recorded_trace_agrees_with_its_events() {
    let paths = recorded("events.jsonl");
    let mut spans = 0;
    for path in &paths {
        let run_dir = path.parent().unwrap();
        spans += traces::agree_with_events(run_dir).len();
        let events = traces::json_lines(path);
        let message = fs::read_to_string(run_dir.join("message.md")).unwrap();
        if let Some(parent) = message.lines().find_map(|l| l.strip_prefix("parent: ")) {
            let parent = traces::json_lines(&run_dir.with_file_name(parent).join("events.jsonl"));
            assert_eq!(
                events[0]["trace_id"],
                parent[0]["trace_id"],
                "{}",
                path.display()
            );
            let traceparent = format!(
                "traceparent: 00-{}-{}-01",
                events[0]["trace_id"].as_str().unwrap(),
                events[0]["parent_span_id"].as_str().unwrap()
            );
            assert!(message.contains(&traceparent), "{}", path.display());
        }
    }
    assert!(spans >= 40, "only {spans} spans");
}

#[test]
fn every_request_and_reply_in_examples_validates() {
    for (file, schema_text) in [
        ("request.json", schema::REQUEST_SCHEMA),
        ("reply.json", schema::REPLY_SCHEMA),
    ] {
        let paths = recorded(file);
        assert!(paths.len() >= 3, "found only {paths:?}");
        let found = json_rejects(schema_text, &read(&paths));
        assert!(found.is_empty(), "{found}");
    }
}

/// Mistakes a router or a pipeline could make: each is rejected.
#[test]
fn wrong_events_requests_and_replies_are_rejected() {
    let events = schema::events_validator();
    let common = r#""v": 1, "seq": 1, "ts": "2026-10-01T14:30:05.123Z", "run_id": "r", "machine": "m", "trigger": "inbox", "trace_id": "4bf92f3577b34da6a3ce929d0e0e4736""#;
    for wrong in [
        // An unknown field, a v2 line, an unknown type, a field of another type.
        r#""type": "run_finished", "state": "done", "duration_ms": 1, "extra": 1"#,
        r#""type": "run_finished", "state": "done", "duration_ms": 1, "v": 2"#,
        r#""type": "finished", "state": "done", "duration_ms": 1"#,
        r#""type": "run_finished", "state": "done", "duration_ms": 1, "cause": "crash""#,
        // A claim without its file; a model decision with an error and a pick.
        r#""type": "transition", "from": null, "event": "claimed", "to": "a", "source": "claim", "exit_code": null"#,
        r#""type": "decision", "state": "s", "kind": "model", "event": "error", "options": ["a"], "router": "router", "duration_ms": 0, "pick": "a", "router_error": "x""#,
        // A person wait without its deadline; a child wait with options.
        r#""type": "waiting", "state": "s", "wait_id": "r.w1", "options": ["a"]"#,
        r#""type": "waiting", "state": "s", "child": "c", "options": ["a"]"#,
        // No trace id, or one in uppercase or all zeros.
        r#""type": "run_finished", "state": "done", "duration_ms": 1, "trace_id": null"#,
        r#""type": "run_finished", "state": "done", "duration_ms": 1, "trace_id": "4BF92F3577B34DA6A3CE929D0E0E4736""#,
        r#""type": "run_finished", "state": "done", "duration_ms": 1, "trace_id": "00000000000000000000000000000000""#,
        // A script without its span; a span on a waiting event; a parent on a non-claim.
        r#""type": "script", "state": "s", "phase": "invoke", "script": "x", "path": "x", "attempt": 1, "started_at": "2026-10-01T14:30:05.123Z", "duration_ms": 1, "exit_code": 0, "log": "0001-s-x.log""#,
        r#""type": "waiting", "state": "s", "child": "c", "span_id": "00f067aa0ba902b7""#,
        r#""type": "transition", "from": "a", "event": "done", "to": "b", "source": "exit_code", "exit_code": 0, "parent_span_id": "00f067aa0ba902b7""#,
    ] {
        // The fields of `wrong` replace the common ones of the same name.
        let mut line: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(&format!("{{{common}}}")).unwrap();
        let fields: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(&format!("{{{wrong}}}")).unwrap();
        line.extend(fields);
        assert!(!events.is_valid(&line.into()), "accepted: {wrong}");
    }
    let reply = schema::validator(schema::REPLY_SCHEMA);
    for wrong in [
        r#"{"reason": "no event"}"#,
        r#"{"event": "a", "confidence": 1.5}"#,
        r#"{"event": "a", "note": "unknown key"}"#,
    ] {
        assert!(
            !reply.is_valid(&serde_json::from_str(wrong).unwrap()),
            "accepted: {wrong}"
        );
    }
}

/// A run made here with a `model` state and a router whose script answers from the request:
/// the `request.json` decree writes, the router's `reply.json`, and every line of both runs'
/// `events.jsonl` validate.
#[test]
fn a_model_run_writes_files_that_validate() {
    let tmp = TempDir::new().unwrap();
    let decree_dir = tmp.path().join(".decree");
    for dir in ["machines", "inbox"] {
        fs::create_dir_all(decree_dir.join(dir)).unwrap();
    }
    fs::write(
        decree_dir.join("machines/ask.yml"),
        "name: ask\n\
         description: Ask the router which way to go.\n\
         initial: read\n\
         states:\n\
         \x20 read:\n\
         \x20   invoke: read\n\
         \x20   transitions: { done: pick }\n\
         \x20 pick:\n\
         \x20   invoke:\n\
         \x20     model: { question: Which way?, min_confidence: 0.5, output: read }\n\
         \x20   transitions:\n\
         \x20     left:   { target: done, description: Go left. }\n\
         \x20     right:  { target: done, description: Go right. }\n\
         \x20     unsure: failed\n\
         \x20 done: { final: true }\n\
         \x20 failed: { final: true }\n",
    )
    .unwrap();
    fs::write(
        decree_dir.join("machines/router.yml"),
        "name: router\n\
         description: Always go left.\n\
         initial: answer\n\
         states:\n\
         \x20 answer:\n\
         \x20   invoke: answer\n\
         \x20   transitions: { done: done }\n\
         \x20 done: { final: true }\n\
         \x20 failed: { final: true }\n",
    )
    .unwrap();
    script(&decree_dir.join("scripts/read"), "echo 'the road forks'\n");
    script(
        &decree_dir.join("scripts/answer"),
        "test -s \"$DECREE_REQUEST\" || exit 1\n\
         printf '%s' '{\"event\": \"left\", \"reason\": \"Left is shorter.\", \"confidence\": 0.9, \"probabilities\": {\"left\": 0.9, \"right\": 0.1}}' > \"$DECREE_REPLY\"\n",
    );
    fs::write(
        decree_dir.join("inbox/go.md"),
        "---\nid: go\nmachine: ask\n---\nPick a way.\n",
    )
    .unwrap();
    let (code, stdout, stderr) = decree(tmp.path(), "process");
    assert_eq!(code, 0, "{stdout}{stderr}");

    let runs = decree_dir.join("runs");
    let child: Vec<PathBuf> = fs::read_dir(&runs)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.join("request.json").is_file())
        .collect();
    assert_eq!(child.len(), 1, "{child:?}");
    let child = &child[0];
    let found = json_rejects(schema::REQUEST_SCHEMA, &read(&[child.join("request.json")]));
    assert!(found.is_empty(), "{found}");
    let found = json_rejects(schema::REPLY_SCHEMA, &read(&[child.join("reply.json")]));
    assert!(found.is_empty(), "{found}");
    let logs = [runs.join("go/events.jsonl"), child.join("events.jsonl")];
    let found = events_rejects(&read(&logs));
    assert!(found.is_empty(), "{found}");
    let parent = fs::read_to_string(&logs[0]).unwrap();
    for kind in ["\"waiting\"", "\"decision\"", "\"run_finished\""] {
        assert!(parent.contains(kind), "no {kind} event in {parent}");
    }
}

/// An executable bash script at `path` running `body`.
fn script(path: &Path, body: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    common::write_script(path, &format!("#!/usr/bin/env bash\n{body}"));
}
